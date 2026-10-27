//! The integration's priority query, run off the browser's thread.
//!
//! Asking a host which sessions it is running can cost far more than opening
//! the index: Herdr's answer takes one command per workspace and a scan of
//! the process table, and on a busy machine that is hundreds of milliseconds.
//! Nothing on screen depends on it — the markers it feeds appear beside
//! results, and there are none until something is typed — so the browser asks
//! for it here and goes on without it.
use agent_history_core::{Priorities, Session};
use std::{
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc,
    },
    thread,
};

/// An integration's priority query in a form that can run on another thread.
/// It is asked again whenever a scan changes the sessions, so it is `FnMut`
/// and may keep whatever it needs between answers.
pub type BackgroundPrioritizer = Box<dyn FnMut(&[Session]) -> Priorities + Send>;

/// Runs one priority query at a time on its own thread.
pub(crate) struct Worker {
    requests: Sender<Arc<[Session]>>,
    answers: Receiver<Priorities>,
}

impl Worker {
    pub(crate) fn spawn(mut prioritize: BackgroundPrioritizer) -> Self {
        let (requests, inbox) = channel::<Arc<[Session]>>();
        let (outbox, answers) = channel();
        thread::spawn(move || {
            for sessions in inbox {
                if outbox.send(prioritize(&sessions)).is_err() {
                    break;
                }
            }
        });
        Self { requests, answers }
    }

    pub(crate) fn request(&self, sessions: Arc<[Session]>) {
        let _ = self.requests.send(sessions);
    }

    /// The newest answer, if one has arrived. Earlier answers are dropped:
    /// each describes the host at its own moment, and only the last is still
    /// worth showing.
    pub(crate) fn take(&self) -> Option<Priorities> {
        let mut latest = None;
        while let Ok(priorities) = self.answers.try_recv() {
            latest = Some(priorities);
        }
        latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::{Agent, SessionId};
    use std::time::Duration;

    fn id(n: u8) -> SessionId {
        SessionId {
            agent: Agent::Claude,
            native_id: format!("00000000-0000-4000-8000-0000000000{n:02}"),
        }
    }

    /// The point of the worker: the query does not run where the browser
    /// draws and reads keys.
    #[test]
    fn the_query_runs_off_the_asking_thread() {
        let asking = thread::current().id();
        let (ran_on, where_it_ran) = channel();
        let worker = Worker::spawn(Box::new(move |sessions: &[Session]| {
            let _ = ran_on.send(thread::current().id());
            sessions.iter().map(|s| (s.id.clone(), 0)).collect()
        }));
        worker.request(Vec::new().into());
        let ran_on = where_it_ran
            .recv_timeout(Duration::from_secs(5))
            .expect("the query ran");
        assert_ne!(ran_on, asking);
    }

    /// Each answer describes the host at its own moment, so an answer that
    /// has been overtaken is dropped rather than shown.
    #[test]
    fn only_the_newest_answer_is_taken() {
        let (requests, _inbox) = channel();
        let (outbox, answers) = channel();
        let worker = Worker { requests, answers };
        outbox.send(Priorities::from_iter([(id(1), 0)])).unwrap();
        outbox.send(Priorities::from_iter([(id(2), 0)])).unwrap();
        let taken = worker.take().expect("an answer");
        assert_eq!(taken.group(&id(2)), Some(0));
        assert_eq!(taken.group(&id(1)), None, "the overtaken answer is dropped");
        assert!(worker.take().is_none(), "nothing is left to take");
    }

    #[test]
    fn nothing_is_taken_before_an_answer_arrives() {
        let worker = Worker::spawn(Box::new(|_: &[Session]| Priorities::default()));
        assert!(worker.take().is_none());
    }
}
