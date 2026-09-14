use super::common::*;
use crate::*;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct CodexAdapter {
    pub roots: Vec<PathBuf>,
}
impl CodexAdapter {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
        }
    }
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self::new([root.into()])
    }
}
impl Default for CodexAdapter {
    fn default() -> Self {
        let root = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join(".codex/sessions"))
            .unwrap_or_else(|| PathBuf::from(".codex/sessions"));
        Self::with_root(root)
    }
}
impl AgentAdapter for CodexAdapter {
    fn agent(&self) -> Agent {
        Agent::Codex
    }
    fn discover(&self) -> Result<Vec<SessionFile>> {
        discover_jsonl(&self.roots)
    }
    fn parse_record(
        &self,
        session: &Session,
        record: &[u8],
        source: SourceRef,
    ) -> Result<ParsedRecord> {
        let v = object(record)?;
        let typ = string(v.get("type"));
        let payload = v.get("payload").unwrap_or(&v);
        let metadata = SessionMetadataPatch {
            native_id: if typ.as_deref() == Some("session_meta") {
                string(payload.get("id").or_else(|| v.get("session_id")))
            } else {
                None
            },
            cwd: string(payload.get("cwd")).map(PathBuf::from),
            started_at: timestamp(v.get("timestamp")),
        };
        // Codex can emit both response_item and event_msg for one message. event_msg is a
        // transport mirror, so only response_item conversational records are indexed.
        let (kind, content) = match (
            typ.as_deref(),
            string(v.get("role").or_else(|| payload.get("role"))).as_deref(),
        ) {
            (Some("response_item"), Some("user")) | (Some("message"), Some("user")) => (
                EventKind::User,
                payload.get("content").or_else(|| v.get("content")),
            ),
            (Some("response_item"), Some("assistant")) | (Some("message"), Some("assistant")) => (
                EventKind::Assistant,
                payload.get("content").or_else(|| v.get("content")),
            ),
            (Some("response_item"), _) if payload.get("output").is_some() => {
                (EventKind::ToolResult, payload.get("output"))
            }
            _ => {
                return Ok(ParsedRecord {
                    metadata,
                    events: vec![],
                })
            }
        };
        let events = content
            .and_then(text)
            .filter(|s| !s.trim().is_empty())
            .map(|text| NormalizedEvent {
                session_id: session.id.clone(),
                kind,
                timestamp: timestamp(v.get("timestamp")),
                source,
                text,
            })
            .into_iter()
            .collect();
        Ok(ParsedRecord { metadata, events })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            id: SessionId::new(Agent::Codex, "fixture"),
            source: source("fixture.jsonl".as_ref(), 1, 0, 0),
            cwd: None,
            repository: None,
            repository_root: None,
            worktree: None,
            branch: None,
            commit: None,
            started_at: None,
            ended_at: None,
            git_observed_at: None,
        }
    }
    #[test]
    fn response_item_is_normalized_once() {
        let r = br#"{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done \u2603"}]}}"#;
        let parsed = CodexAdapter::with_root(".")
            .parse_record(
                &session(),
                r,
                source("fixture.jsonl".as_ref(), 1, 0, r.len()),
            )
            .unwrap();
        assert_eq!(parsed.events.len(), 1);
        assert_eq!(parsed.events[0].text, "Done ☃");
    }
    #[test]
    fn event_mirror_is_skipped() {
        let r = br#"{"type":"event_msg","payload":{"type":"agent_message","message":"duplicate"}}"#;
        assert!(CodexAdapter::with_root(".")
            .parse_record(&session(), r, source("x".as_ref(), 1, 0, r.len()))
            .unwrap()
            .events
            .is_empty());
    }
}
