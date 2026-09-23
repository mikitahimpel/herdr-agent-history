//! Per-session availability markers. Checking the filesystem can be slow
//! (a recorded path may sit on an unmounted network volume), so it runs on a
//! worker thread, once per session, and the UI only reads the cache.
use agent_history_core::{availability::Availability, SearchResult, Session, SessionId};
use std::{
    collections::{HashMap, HashSet},
    sync::mpsc::{channel, Receiver, Sender},
    thread,
};

/// What a result row shows about its session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SessionState {
    /// The host already runs this session; opening it focuses that agent.
    Live,
    Stored(Availability),
}

/// Availability known so far, keyed by session.
#[derive(Clone, Debug, Default)]
pub struct Availabilities {
    known: HashMap<SessionId, Availability>,
    requested: HashSet<SessionId>,
    live: HashSet<SessionId>,
}

impl Availabilities {
    pub fn set_live(&mut self, ids: impl IntoIterator<Item = SessionId>) {
        self.live = ids.into_iter().collect();
    }

    /// Sessions in `results` not yet asked for; they are marked as asked.
    /// Pure bookkeeping, cheap enough for the keystroke path.
    pub fn wanted(&mut self, results: &[SearchResult]) -> Vec<SessionId> {
        let mut out = Vec::new();
        for r in results {
            if self.requested.insert(r.session_id.clone()) {
                out.push(r.session_id.clone());
            }
        }
        out
    }

    pub fn record(&mut self, id: SessionId, availability: Availability) {
        self.known.insert(id, availability);
    }

    /// `None` while the check is still pending.
    pub fn state(&self, id: &SessionId) -> Option<SessionState> {
        if self.live.contains(id) {
            return Some(SessionState::Live);
        }
        self.known.get(id).copied().map(SessionState::Stored)
    }

    pub fn pending(&self) -> bool {
        self.requested.len() > self.known.len()
    }
}

/// Classifies sessions off the UI thread.
pub(crate) struct Worker {
    requests: Sender<Vec<SessionId>>,
    answers: Receiver<Vec<(SessionId, Availability)>>,
}

impl Worker {
    /// `sessions` is the session table, loaded once; `classify` is
    /// `agent_history_core::availability::availability` outside tests.
    pub(crate) fn spawn(
        sessions: Vec<Session>,
        classify: impl Fn(&Session) -> Availability + Send + 'static,
    ) -> Self {
        let (requests, inbox) = channel::<Vec<SessionId>>();
        let (outbox, answers) = channel();
        thread::spawn(move || {
            let sessions: HashMap<SessionId, Session> =
                sessions.into_iter().map(|s| (s.id.clone(), s)).collect();
            for ids in inbox {
                let batch: Vec<_> = ids
                    .into_iter()
                    .map(|id| {
                        let a = sessions
                            .get(&id)
                            .map_or(Availability::TranscriptOnly, &classify);
                        (id, a)
                    })
                    .collect();
                if outbox.send(batch).is_err() {
                    break;
                }
            }
        });
        Self { requests, answers }
    }

    pub(crate) fn request(&self, ids: Vec<SessionId>) {
        if !ids.is_empty() {
            let _ = self.requests.send(ids);
        }
    }

    /// Moves finished answers into `into` without blocking.
    pub(crate) fn drain(&self, into: &mut Availabilities) {
        while let Ok(batch) = self.answers.try_recv() {
            for (id, a) in batch {
                into.record(id, a);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_history_core::Agent;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::{Duration, Instant},
    };

    fn id(n: u8) -> SessionId {
        SessionId {
            agent: Agent::Claude,
            native_id: format!("00000000-0000-4000-8000-0000000000{n:02}"),
        }
    }

    #[test]
    fn live_overrides_stored_and_unknown_is_pending() {
        let mut a = Availabilities::default();
        assert_eq!(a.state(&id(1)), None);
        a.record(id(1), Availability::OnDisk);
        assert_eq!(
            a.state(&id(1)),
            Some(SessionState::Stored(Availability::OnDisk))
        );
        a.set_live([id(1)]);
        assert_eq!(a.state(&id(1)), Some(SessionState::Live));
    }

    #[test]
    fn worker_classifies_each_session_once_off_thread() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let worker = Worker::spawn(Vec::new(), move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Availability::OnDisk
        });
        let mut a = Availabilities::default();
        worker.request(vec![id(1), id(2)]);
        let deadline = Instant::now() + Duration::from_secs(5);
        while a.pending_for(&[id(1), id(2)]) && Instant::now() < deadline {
            worker.drain(&mut a);
            std::thread::yield_now();
        }
        // Unknown IDs are answered without calling the classifier.
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            a.state(&id(2)),
            Some(SessionState::Stored(Availability::TranscriptOnly))
        );
    }

    impl Availabilities {
        fn pending_for(&self, ids: &[SessionId]) -> bool {
            ids.iter().any(|i| !self.known.contains_key(i))
        }
    }
}
