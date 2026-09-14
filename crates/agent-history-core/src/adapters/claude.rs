use super::common::*;
use crate::*;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ClaudeAdapter {
    pub roots: Vec<PathBuf>,
}
impl ClaudeAdapter {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            roots: roots.into_iter().collect(),
        }
    }
    pub fn with_root(root: impl Into<PathBuf>) -> Self {
        Self::new([root.into()])
    }
}
impl Default for ClaudeAdapter {
    fn default() -> Self {
        let root = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join(".claude"))
            .unwrap_or_else(|| PathBuf::from(".claude"));
        Self::with_root(root)
    }
}
impl AgentAdapter for ClaudeAdapter {
    fn agent(&self) -> Agent {
        Agent::Claude
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
        let native = string(v.get("sessionId").or_else(|| v.get("session_id")));
        let cwd = string(v.get("cwd")).map(PathBuf::from);
        let metadata = SessionMetadataPatch {
            native_id: native,
            cwd,
            started_at: timestamp(v.get("timestamp")),
        };
        let mut kind = match string(v.get("type")).as_deref() {
            Some("user") => EventKind::User,
            Some("assistant") => EventKind::Assistant,
            _ => {
                return Ok(ParsedRecord {
                    metadata,
                    events: vec![],
                })
            }
        };
        let msg = v.get("message").unwrap_or(&v);
        let content = msg.get("content").unwrap_or(msg);
        if kind == EventKind::User
            && content.as_array().is_some_and(|blocks| {
                !blocks.is_empty()
                    && blocks
                        .iter()
                        .all(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
            })
        {
            kind = EventKind::ToolResult;
        }
        let mut events = Vec::new();
        if let Some(s) = text(content) {
            if !s.trim().is_empty() {
                events.push(NormalizedEvent {
                    session_id: session.id.clone(),
                    kind,
                    timestamp: timestamp(v.get("timestamp")),
                    source,
                    text: s,
                });
            }
        }
        Ok(ParsedRecord { metadata, events })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> Session {
        Session {
            id: SessionId::new(Agent::Claude, "fixture"),
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
    fn extracts_text_and_skips_protocol_fields() {
        let r = br#"{"type":"assistant","sessionId":"s","message":{"role":"assistant","content":[{"type":"text","text":"Unicode \u2603 answer"},{"type":"tool_use","id":"secret","input":{"password":"no"}}]}}"#;
        let parsed = ClaudeAdapter::with_root(".")
            .parse_record(
                &session(),
                r,
                source("fixture.jsonl".as_ref(), 1, 0, r.len()),
            )
            .unwrap();
        assert_eq!(parsed.events[0].text, "Unicode ☃ answer");
        assert!(!parsed.events[0].text.contains("secret"));
    }
    #[test]
    fn malformed_is_an_error() {
        assert!(ClaudeAdapter::with_root(".")
            .parse_record(&session(), b"{", source("x".as_ref(), 1, 0, 1))
            .is_err());
    }
}
