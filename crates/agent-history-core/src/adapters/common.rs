use crate::{CoreError, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn discover_jsonl(roots: &[PathBuf]) -> Result<Vec<crate::SessionFile>> {
    let mut paths = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        // A single inaccessible directory must not hide sessions below other roots.
        let _ = walk(root, &mut paths);
    }
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| {
            Ok(crate::SessionFile {
                path,
                file_id: 0,
                generation: 0,
            })
        })
        .collect()
}
fn walk(path: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_file() {
        if path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            out.push(path.to_path_buf());
        }
        return Ok(());
    }
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry.map_err(CoreError::Io)?;
            walk(&entry.path(), out)?;
        }
    }
    Ok(())
}
pub(crate) fn object(record: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(record).map_err(|e| CoreError::InvalidRecord(e.to_string()))
}
pub(crate) fn string(v: Option<&serde_json::Value>) -> Option<String> {
    v.and_then(|x| x.as_str()).map(ToOwned::to_owned)
}
pub(crate) fn timestamp(v: Option<&serde_json::Value>) -> Option<std::time::SystemTime> {
    let dt = chrono::DateTime::parse_from_rfc3339(v?.as_str()?).ok()?;
    if dt.timestamp() < 0 {
        return None;
    }
    Some(
        std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(dt.timestamp() as u64)
            + std::time::Duration::from_nanos(dt.timestamp_subsec_nanos() as u64),
    )
}
pub(crate) fn text(v: &serde_json::Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_owned());
    }
    if let Some(a) = v.as_array() {
        let s = a
            .iter()
            .filter_map(|x| {
                let kind = x.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if matches!(kind, "text" | "input_text" | "output_text") {
                    x.get("text")
                        .and_then(|t| t.as_str())
                        .map(ToOwned::to_owned)
                } else if kind == "tool_result" {
                    x.get("content").and_then(text)
                } else {
                    None
                }
            })
            .collect::<Vec<String>>()
            .join("\n");
        if !s.is_empty() {
            return Some(s);
        }
    }
    None
}
#[cfg(test)]
pub(crate) fn source(path: &Path, file_id: u64, generation: u64, len: usize) -> crate::SourceRef {
    crate::SourceRef::new(path, file_id, generation, 0..len as u64).expect("valid range")
}
