//! Bounded normalized conversation from the canonical source.
use crate::{Agent, AgentAdapter, CoreError, IndexStore, Result, SourceRef};
use std::os::unix::fs::MetadataExt;
use std::{
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom},
};
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preview {
    pub source: SourceRef,
    pub text: String,
    pub truncated_before: bool,
    pub truncated_after: bool,
}
/// Verify the indexed generation and read complete records in a bounded window.
/// Context and total read allocation are capped regardless of the source range.
pub fn preview_source<S: IndexStore>(
    store: &S,
    source: &SourceRef,
    context: u64,
) -> Result<Preview> {
    let (indexed, bytes) = store.indexed_file_state(&source.path)?.ok_or_else(stale)?;
    if indexed.file_id != source.file_id
        || indexed.generation != source.generation
        || source.byte_range.end > indexed.committed_offset
    {
        return Err(stale());
    }
    let checkpoint = crate::index::decode(bytes.as_deref().ok_or_else(stale)?)?;
    let mut file = File::open(&source.path)?;
    let before = file.metadata()?;
    // Any unindexed mutation requires indexing before preview; append is safe once indexed.
    if before.ctime() != checkpoint.ctime
        || before.ctime_nsec() != checkpoint.ctime_nsec
        || before.len() != indexed.size
        || before.modified().ok() != indexed.modified
        || !crate::index::verify(&mut file, &checkpoint)?
    {
        return Err(stale());
    }
    let context = context.min(1024 * 1024);
    let start = source.byte_range.start.saturating_sub(context);
    let end = source
        .byte_range
        .end
        .saturating_add(context)
        .saturating_add(1)
        .min(indexed.committed_offset)
        .min(start.saturating_add(8 * 1024 * 1024));
    file.seek(SeekFrom::Start(start.saturating_sub(1)))?;
    let mut reader = BufReader::new(file.take(end - start + u64::from(start > 0)));
    let mut cursor = start;
    if start > 0 {
        let mut previous = [0];
        reader.read_exact(&mut previous)?;
        if previous[0] != b'\n' {
            let (_, n, _) = crate::index::record(&mut reader, 0)?;
            cursor += n;
        }
    }
    let adapter: Box<dyn AgentAdapter> = match checkpoint.session.id.agent {
        Agent::Claude => Box::new(crate::adapters::ClaudeAdapter::new([])),
        Agent::Codex => Box::new(crate::adapters::CodexAdapter::new([])),
    };
    let mut text = String::new();
    loop {
        let (line, n, complete) =
            crate::index::record(&mut reader, crate::index::DEFAULT_MAX_RECORD_BYTES)?;
        if !complete {
            break;
        }
        if n <= crate::index::DEFAULT_MAX_RECORD_BYTES as u64 + 1 {
            let range = SourceRef::new(
                &source.path,
                source.file_id,
                source.generation,
                cursor..cursor + n - 1,
            )
            .unwrap();
            if let Ok(parsed) =
                adapter.parse_record(&checkpoint.session, &line[..line.len() - 1], range)
            {
                for event in parsed.events {
                    if !text.is_empty() {
                        text.push_str("\n\n")
                    }
                    text.push_str(match event.kind {
                        crate::EventKind::User => "User: ",
                        crate::EventKind::Assistant => "Assistant: ",
                        crate::EventKind::ToolResult => "Tool: ",
                    });
                    text.push_str(&sanitize(&event.text));
                }
            }
        }
        cursor += n;
    }
    let file = reader.into_inner().into_inner();
    if !crate::index::same(&before, &file.metadata()?)
        || !crate::index::same(&before, &fs::metadata(&source.path)?)
    {
        return Err(stale());
    }
    Ok(Preview {
        source: SourceRef::new(
            &source.path,
            source.file_id,
            source.generation,
            start..cursor,
        )
        .unwrap(),
        text,
        truncated_before: start > 0,
        truncated_after: cursor < indexed.committed_offset,
    })
}
fn stale() -> CoreError {
    CoreError::Unsupported("source generation is stale; index again".into())
}
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                '�'
            } else {
                c
            }
        })
        .collect()
}
