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
pub struct SessionIdState {
    pub agent: String,
    pub native_id: String,
}

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
        let pending = if bytes.is_empty() {
            None
        } else {
            Some(
                serde_json::from_slice(bytes)
                    .map_err(|e| crate::CoreError::InvalidRecord(e.to_string()))?,
            )
        };
        Ok(Self {
            max_bytes: max_bytes.max(4),
            pending,
            next_ordinal: 0,
        })
    }
    pub fn state(&self) -> Result<Vec<u8>> {
        self.pending
            .as_ref()
            .map(|s| {
                serde_json::to_vec(s).map_err(|e| crate::CoreError::InvalidRecord(e.to_string()))
            })
            .unwrap_or_else(|| Ok(Vec::new()))
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
                && p.source_path == event.source.path.to_string_lossy();
            if !same {
                let old = self.pending.take().unwrap();
                let mut out = Self::emit(old);
                out.extend(self.push(event));
                return out;
            }
        }
        if event.kind == EventKind::User
            && self.pending.as_ref().is_some_and(|p| !p.text.is_empty())
        {
            if let Some(p) = self.pending.take() {
                out.extend(Self::emit(p));
            }
        }
        let p = self.pending.get_or_insert_with(|| OpenTurnState {
            session_id: SessionIdState {
                agent: format!("{:?}", event.session_id.agent),
                native_id: event.session_id.native_id.clone(),
            },
            ordinal: self.next_ordinal,
            timestamp_millis: None,
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
                out.extend(Self::emit(q));
                self.next_ordinal += 1;
            }
            self.pending = None;
        }
        out
    }
    pub fn finish(&mut self) -> Vec<ConversationChunk> {
        self.pending.take().map(Self::emit).unwrap_or_default()
    }
    fn emit(p: OpenTurnState) -> Vec<ConversationChunk> {
        let agent = match p.session_id.agent.as_str() {
            "Claude" => crate::Agent::Claude,
            _ => crate::Agent::Codex,
        };
        let source = SourceRef::new(p.source_path, p.file_id, p.generation, p.start..p.end)
            .expect("builder range");
        vec![ConversationChunk {
            session_id: SessionId::new(agent, p.session_id.native_id),
            ordinal: p.ordinal,
            timestamp: None,
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
