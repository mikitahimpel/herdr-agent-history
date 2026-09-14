//! Deterministic conversation turn grouping with a small persistable open-turn state.
use crate::{ConversationChunk, EventKind, NormalizedEvent, Result, SessionId, SourceRef};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MAX_CHUNK_BYTES: usize = 16 * 1024;
pub const MAX_TOOL_OUTPUT_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenTurnState {
    pub session_id: SessionIdState,
    pub ordinal: u64,
    pub timestamp_millis: Option<i128>,
    pub source_path: String,
    pub file_id: u64,
    pub generation: u64,
    pub start: u64,
    pub end: u64,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct BuilderState {
    next_ordinal: u64,
    pending: Option<OpenTurnState>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionIdState {
    pub agent: String,
    pub native_id: String,
}

#[derive(Clone)]
pub struct ChunkBuilder {
    max_bytes: usize,
    pending: Option<OpenTurnState>,
    next_ordinal: u64,
}
impl ChunkBuilder {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            max_bytes: max_bytes.max(4),
            pending: None,
            next_ordinal: 0,
        }
    }
    pub fn with_default_limit() -> Self {
        Self::new(DEFAULT_MAX_CHUNK_BYTES)
    }
    pub fn from_state(max_bytes: usize, bytes: &[u8]) -> Result<Self> {
        let state: BuilderState = if bytes.is_empty() {
            BuilderState {
                next_ordinal: 0,
                pending: None,
            }
        } else {
            serde_json::from_slice(bytes)
                .map_err(|e| crate::CoreError::InvalidRecord(e.to_string()))?
        };
        if state.next_ordinal > 1_000_000_000
            || state.pending.as_ref().is_some_and(|p| {
                p.text.len() > max_bytes.max(4) * 2
                    || p.start > p.end
                    || p.ordinal != state.next_ordinal
                    || p.timestamp_millis
                        .is_some_and(|v| v < 0 || v > i64::MAX as i128)
                    || !matches!(p.session_id.agent.as_str(), "Claude" | "Codex")
            })
        {
            return Err(crate::CoreError::InvalidRecord(
                "invalid open turn state".into(),
            ));
        }
        Ok(Self {
            max_bytes: max_bytes.max(4),
            pending: state.pending,
            next_ordinal: state.next_ordinal,
        })
    }
    pub fn state(&self) -> Result<Vec<u8>> {
        serde_json::to_vec(&BuilderState {
            next_ordinal: self.next_ordinal,
            pending: self.pending.clone(),
        })
        .map_err(|e| crate::CoreError::InvalidRecord(e.to_string()))
    }
    pub fn push(&mut self, event: NormalizedEvent) -> Vec<ConversationChunk> {
        let mut out = Vec::new();
        if event.text.trim().is_empty() {
            return out;
        }
        let text = normalize(&event.text);
        let text = if event.kind == EventKind::ToolResult {
            truncate_bytes(&text, MAX_TOOL_OUTPUT_BYTES)
        } else {
            text
        };
        if let Some(p) = &self.pending {
            let same = p.file_id == event.source.file_id
                && p.generation == event.source.generation
                && p.source_path == event.source.path.to_string_lossy()
                && p.session_id.native_id == event.session_id.native_id
                && p.session_id.agent == format!("{:?}", event.session_id.agent);
            if !same {
                let mut out = self.finish();
                out.extend(self.push(event));
                return out;
            }
        }
        if event.kind == EventKind::User
            && self.pending.as_ref().is_some_and(|p| !p.text.is_empty())
        {
            out.extend(self.finish());
        }
        let p = self.pending.get_or_insert_with(|| OpenTurnState {
            session_id: SessionIdState {
                agent: format!("{:?}", event.session_id.agent),
                native_id: event.session_id.native_id.clone(),
            },
            ordinal: self.next_ordinal,
            timestamp_millis: event
                .timestamp
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i128),
            source_path: event.source.path.to_string_lossy().into_owned(),
            file_id: event.source.file_id,
            generation: event.source.generation,
            start: event.source.byte_range.start,
            end: event.source.byte_range.end,
            text: String::new(),
        });
        if !p.text.is_empty() {
            p.text.push('\n');
        }
        p.text.push_str(&text);
        p.end = p.end.max(event.source.byte_range.end);
        p.start = p.start.min(event.source.byte_range.start);
        if p.text.len() >= self.max_bytes {
            let full = p.text.clone();
            p.text.clear();
            for part in split_utf8(&full, self.max_bytes) {
                let mut q = p.clone();
                q.text = part.to_owned();
                q.ordinal = self.next_ordinal;
                self.next_ordinal += 1;
                out.extend(Self::emit(q));
            }
            self.pending = None;
        }
        out
    }
    /// Current searchable open turn, without closing it at an indexing boundary.
    pub fn snapshot(&self) -> Option<ConversationChunk> {
        self.pending.clone().and_then(|p| Self::emit(p).pop())
    }

    pub fn finish(&mut self) -> Vec<ConversationChunk> {
        self.pending
            .take()
            .map(|mut p| {
                p.ordinal = self.next_ordinal;
                self.next_ordinal += 1;
                Self::emit(p)
            })
            .unwrap_or_default()
    }
    fn emit(p: OpenTurnState) -> Vec<ConversationChunk> {
        let agent = match p.session_id.agent.as_str() {
            "Claude" => crate::Agent::Claude,
            _ => crate::Agent::Codex,
        };
        let source = match SourceRef::new(p.source_path, p.file_id, p.generation, p.start..p.end) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        vec![ConversationChunk {
            session_id: SessionId::new(agent, p.session_id.native_id),
            ordinal: p.ordinal,
            timestamp: p.timestamp_millis.and_then(|m| {
                if m >= 0 {
                    Some(std::time::UNIX_EPOCH + std::time::Duration::from_millis(m as u64))
                } else {
                    None
                }
            }),
            source,
            text: p.text,
        }]
    }
}
impl Default for ChunkBuilder {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_CHUNK_BYTES)
    }
}
fn normalize(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_owned()
    } else {
        format!("{}…", &s[..s.floor_char_boundary(max.saturating_sub(3))])
    }
}
fn split_utf8(s: &str, max: usize) -> impl Iterator<Item = &str> {
    let mut at = 0;
    std::iter::from_fn(move || {
        if at >= s.len() {
            return None;
        }
        let end = (at + max).min(s.len());
        let mut e = end;
        while !s.is_char_boundary(e) {
            e -= 1;
        }
        let r = &s[at..e];
        at = e;
        Some(r)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ev(id: &str, kind: EventKind, text: &str, start: u64) -> NormalizedEvent {
        NormalizedEvent {
            session_id: SessionId::new(crate::Agent::Claude, id),
            kind,
            timestamp: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(7)),
            source: SourceRef::new("f", 1, 0, start..start + text.len() as u64).unwrap(),
            text: text.into(),
        }
    }
    #[test]
    fn ordinals_are_unique_across_turns_and_splits() {
        let mut b = ChunkBuilder::new(8);
        let mut out = b.push(ev("s", EventKind::User, "abcdefghijk", 0));
        out.extend(b.push(ev("s", EventKind::User, "next", 20)));
        out.extend(b.finish());
        assert_eq!(
            out.iter().map(|c| c.ordinal).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }
    #[test]
    fn state_round_trips_pending_and_empty() {
        let mut b = ChunkBuilder::default();
        let empty = b.state().unwrap();
        let _ = ChunkBuilder::from_state(10, &empty).unwrap();
        b.push(ev("s", EventKind::User, "pending", 0));
        let bytes = b.state().unwrap();
        let mut r = ChunkBuilder::from_state(100, &bytes).unwrap();
        assert_eq!(r.finish()[0].text, "pending");
    }
    #[test]
    fn unicode_small_limit_makes_progress() {
        let mut b = ChunkBuilder::new(4);
        let out = b.push(ev("s", EventKind::User, "☃☃", 0));
        assert!(!out.is_empty());
    }
    #[test]
    fn mismatched_source_flushes() {
        let mut b = ChunkBuilder::default();
        b.push(ev("s", EventKind::User, "one", 0));
        let mut e = ev("s", EventKind::Assistant, "two", 20);
        e.source = SourceRef::new("g", 2, 1, 20..23).unwrap();
        assert_eq!(b.push(e).len(), 1);
    }
    #[test]
    fn corrupt_state_is_error() {
        assert!(ChunkBuilder::from_state(10, br#"{"next_ordinal":1,"pending":{"session_id":{"agent":"Other","native_id":"x"},"ordinal":0,"timestamp_millis":null,"source_path":"x","file_id":1,"generation":0,"start":4,"end":1,"text":"x"}}"#).is_err());
    }
    #[test]
    fn ordinary_turns_have_distinct_ordinals() {
        let mut b = ChunkBuilder::default();
        let mut chunks = Vec::new();
        for text in ["one", "two", "three"] {
            chunks.extend(b.push(ev("s", EventKind::User, text, 0)));
        }
        chunks.extend(b.finish());
        assert_eq!(
            chunks.iter().map(|c| c.ordinal).collect::<Vec<_>>(),
            [0, 1, 2]
        );
    }
    #[test]
    fn every_restart_boundary_matches_whole_file() {
        let events = vec![
            ev("s", EventKind::User, "remember topic", 0),
            ev("s", EventKind::Assistant, "a useful answer ☃", 30),
            ev("s", EventKind::ToolResult, "tool output", 60),
            ev("s", EventKind::User, "another turn", 90),
            ev("s", EventKind::Assistant, "last answer", 120),
        ];
        let collect = |restart: bool| {
            let mut b = ChunkBuilder::new(24);
            let mut chunks = Vec::new();
            for event in events.clone() {
                chunks.extend(b.push(event));
                if restart {
                    let before = b.snapshot();
                    b = ChunkBuilder::from_state(24, &b.state().unwrap()).unwrap();
                    assert_eq!(before, b.snapshot());
                }
            }
            chunks.extend(b.finish());
            chunks
        };
        assert_eq!(collect(false), collect(true));
    }
    #[test]
    fn same_source_different_session_is_not_merged() {
        let mut b = ChunkBuilder::default();
        b.push(ev("one", EventKind::User, "first", 0));
        let chunks = b.push(ev("two", EventKind::Assistant, "second", 20));
        assert_eq!(chunks[0].session_id.native_id, "one");
        assert_eq!(b.snapshot().unwrap().session_id.native_id, "two");
    }
    #[test]
    fn snapshot_keeps_turn_open_and_timestamp() {
        let mut b = ChunkBuilder::default();
        b.push(ev("s", EventKind::User, "question", 0));
        let snapshot = b.snapshot().unwrap();
        assert_eq!(
            snapshot.timestamp,
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(7))
        );
        let mut b = ChunkBuilder::from_state(DEFAULT_MAX_CHUNK_BYTES, &b.state().unwrap()).unwrap();
        assert!(b
            .push(ev("s", EventKind::Assistant, "answer", 20))
            .is_empty());
        let current = b.snapshot().unwrap();
        assert_eq!(current.ordinal, snapshot.ordinal);
        assert_eq!(current.text, "question\nanswer");
        assert_eq!(current.source.byte_range, 0..26);
    }
}
