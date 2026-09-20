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
        for entry in fs::read_dir(path)?.flatten() {
            let _ = walk(&entry.path(), out);
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
pub(crate) fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}
/// Recorded branch names. Agents write the literal `HEAD` for a detached checkout,
/// which is not a branch and must not be presented as one.
pub(crate) fn branch_name(v: Option<String>) -> Option<String> {
    non_empty(v).filter(|s| s != "HEAD")
}
/// Derives `owner/name` from a recorded remote URL for display. The URL itself is
/// persisted unchanged; this is a label, never a local path.
pub(crate) fn repository_label(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let (path, hosted) = match url.split_once("://") {
        Some((_, rest)) => (rest.split_once('/').map_or("", |(_, p)| p), true),
        None => match url.split_once(':') {
            // scp-like syntax, `git@host:owner/name`, only when the colon precedes any slash.
            Some((host, rest)) if !host.contains('/') => (rest, true),
            _ => (url, false),
        },
    };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let keep = if hosted { 2 } else { 1 };
    let label = segments[segments.len().saturating_sub(keep)..].join("/");
    (!label.is_empty()).then_some(label)
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_tool_results_and_text_blocks_exclude_protocol() {
        let v = serde_json::json!([
            {"type":"text","text":"first"},
            {"type":"tool_use","text":"hidden","input":{"command":"secret"}},
            {"type":"tool_result","content":[{"type":"text","text":"output"}]},
            {"type":"text","text":"last"}
        ]);
        assert_eq!(text(&v).as_deref(), Some("first\nlast"));
    }
    #[test]
    fn discovery_is_sorted_deduplicated_and_skips_symlinks() {
        let dir = crate::test_support::TempDir::new("discovery").unwrap();
        let a = dir.jsonl("a.jsonl", &["{}"]).unwrap();
        dir.jsonl("z.jsonl", &["{}"]).unwrap();
        std::os::unix::fs::symlink(dir.path(), dir.path().join("loop")).unwrap();
        let found = discover_jsonl(&[dir.path().to_owned(), a]).unwrap();
        assert_eq!(found.len(), 2);
        assert!(found[0].path < found[1].path);
    }
    #[test]
    fn repository_labels_come_only_from_recorded_urls() {
        for (url, expected) in [
            (
                "git@github.com:mikitahimpel/memoxia.git",
                Some("mikitahimpel/memoxia"),
            ),
            ("https://github.com/owner/name.git", Some("owner/name")),
            ("ssh://git@github.com/owner/name", Some("owner/name")),
            ("https://example.invalid/a/b/c/name.git/", Some("c/name")),
            ("/srv/mirrors/name.git", Some("name")),
            ("", None),
            ("   ", None),
        ] {
            assert_eq!(repository_label(url).as_deref(), expected, "{url}");
        }
    }
    #[test]
    fn detached_head_is_not_a_recorded_branch() {
        assert_eq!(branch_name(Some("HEAD".into())), None);
        assert_eq!(branch_name(Some(" ".into())), None);
        assert_eq!(branch_name(Some("main".into())).as_deref(), Some("main"));
    }
    #[test]
    fn timestamp_preserves_offset_and_subseconds() {
        let value = serde_json::json!("2026-09-14T21:01:00.123456789+02:00");
        let utc = serde_json::json!("2026-09-14T19:01:00.123456789Z");
        assert_eq!(timestamp(Some(&value)), timestamp(Some(&utc)));
        assert_eq!(
            timestamp(Some(&value))
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos(),
            123456789
        );
    }
}
