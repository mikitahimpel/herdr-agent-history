use agent_history_core::adapters::{ClaudeAdapter, CodexAdapter};
use agent_history_core::{Agent, AgentAdapter, EventKind, SqliteStore};
use std::env;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("agent-history: {}", sanitize(&message));
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    if args.is_empty() || (args.len() == 1 && (args[0] == "--help" || args[0] == "-h")) {
        print_help();
        return Ok(());
    }
    if args.len() == 1 && (args[0] == "--version" || args[0] == "-V") {
        println!("agent-history {VERSION}");
        return Ok(());
    }
    let command = args[0].as_str();
    let options = Options::parse(&args[1..])?;
    if matches!(command, "index" | "status") && !options.positional.is_empty() {
        return Err(format!("{command} does not accept positional arguments"));
    }
    let db = options.db.clone().map_or_else(default_db, Ok)?;
    match command {
        "index" => index(db, options),
        "search" => search(db, options),
        "status" => status(db),
        "preview" => preview(db, options),
        other => Err(format!("unknown command '{other}'; try --help")),
    }
}

#[derive(Default)]
struct Options {
    db: Option<PathBuf>,
    claude_root: Option<PathBuf>,
    codex_root: Option<PathBuf>,
    query: Vec<String>,
    positional: Vec<String>,
    role: Option<EventKind>,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut o = Self::default();
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a == "--role" {
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| "--role requires all, user, or assistant".to_string())?;
                o.role = match value.to_ascii_lowercase().as_str() {
                    "all" => None,
                    "user" => Some(EventKind::User),
                    "assistant" => Some(EventKind::Assistant),
                    _ => return Err("--role must be all, user, or assistant".into()),
                };
                i += 1;
                continue;
            }
            let target = match a.as_str() {
                "--db" => &mut o.db,
                "--claude-root" => &mut o.claude_root,
                "--codex-root" => &mut o.codex_root,
                a if a.starts_with('-') => return Err(format!("unknown option '{a}'")),
                _ => {
                    o.positional.push(a.clone());
                    i += 1;
                    continue;
                }
            };
            i += 1;
            let value = args.get(i).ok_or_else(|| format!("{a} requires a value"))?;
            if value.starts_with('-') {
                return Err(format!("{a} requires a value"));
            }
            *target = Some(PathBuf::from(value));
            i += 1;
        }
        o.query = o.positional.clone();
        Ok(o)
    }
}
fn open(path: &Path) -> Result<SqliteStore, String> {
    SqliteStore::open(path).map_err(|e| sanitize(&e.to_string()))
}
fn adapters(o: &Options) -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(
            o.claude_root
                .clone()
                .map_or_else(ClaudeAdapter::default, ClaudeAdapter::with_root),
        ),
        Box::new(
            o.codex_root
                .clone()
                .map_or_else(CodexAdapter::default, CodexAdapter::with_root),
        ),
    ]
}
fn index(db: PathBuf, o: Options) -> Result<(), String> {
    let mut store = open(&db)?;
    let r = agent_history_core::index::index_all(&mut store, &adapters(&o))
        .map_err(|e| sanitize(&e.to_string()))?;
    println!(
        "indexed {} files ({} failed), {} records ({} bytes, {} chunks; {} malformed)",
        r.files, r.failed_files, r.records, r.bytes_read, r.chunks, r.malformed_records
    );
    for error in &r.errors {
        eprintln!("agent-history: indexing error: {}", sanitize(error));
    }
    if r.failed_files > 0 {
        return Err(format!(
            "index completed with {} failed file(s)",
            r.failed_files
        ));
    }
    Ok(())
}
fn search(db: PathBuf, o: Options) -> Result<(), String> {
    if o.query.is_empty() {
        return Err("search requires a query".into());
    }
    let store = open(&db)?;
    let results = store
        .search_with_role(&o.query.join(" "), 50, o.role)
        .map_err(|e| sanitize(&e.to_string()))?;
    for (i, r) in results.iter().enumerate() {
        let agent = match r.agent {
            Agent::Claude => "Claude",
            Agent::Codex => "Codex",
        };
        println!(
            "{}\t{}\t{}\t{} / {}\t{}\t{}\t{}:{}-{}\t{}",
            i + 1,
            agent,
            role_name(r.kind),
            sanitize(r.repository.as_deref().unwrap_or("-")),
            sanitize(r.branch.as_deref().unwrap_or("-")),
            sanitize(&r.session_id.native_id),
            r.timestamp
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
                .unwrap_or_else(|| "-".into()),
            sanitize(&r.source.path.display().to_string()),
            r.source.byte_range.start,
            r.source.byte_range.end,
            sanitize(&r.snippet)
        );
    }
    Ok(())
}
fn status(db: PathBuf) -> Result<(), String> {
    let store = open(&db)?;
    let s = store.status().map_err(|e| sanitize(&e.to_string()))?;
    let bytes = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
    let mut claude = 0;
    let mut codex = 0;
    for session in store.sessions().map_err(|e| sanitize(&e.to_string()))? {
        match session.id.agent {
            Agent::Claude => claude += 1,
            Agent::Codex => codex += 1,
        }
    }
    println!(
        "files: {}\nsessions: {} (Claude: {}, Codex: {})\nchunks: {}\ndatabase: {}\ndatabase bytes: {}",
        s.files, s.sessions, claude, codex, s.chunks, sanitize(&db.display().to_string()), bytes
    );
    Ok(())
}
fn preview(db: PathBuf, o: Options) -> Result<(), String> {
    if o.positional.len() != 2 {
        return Err("preview requires <claude|codex> <session-id>".into());
    }
    let agent = match o.positional[0].to_ascii_lowercase().as_str() {
        "claude" => Agent::Claude,
        "codex" => Agent::Codex,
        _ => return Err("preview agent must be claude or codex".into()),
    };
    let store = open(&db)?;
    let id = agent_history_core::SessionId::new(agent, &o.positional[1]);
    let session = store
        .session(&id)
        .map_err(|e| sanitize(&e.to_string()))?
        .ok_or_else(|| "session not found".to_string())?;
    let p = agent_history_core::preview::preview_source(&store, &session.source, 4096)
        .map_err(|e| sanitize(&e.to_string()))?;
    println!(
        "agent: {:?}\nsession: {}\nsource: {}:{}-{}\n{}",
        agent,
        sanitize(&id.native_id),
        sanitize(&p.source.path.display().to_string()),
        p.source.byte_range.start,
        p.source.byte_range.end,
        p.text
    );
    Ok(())
}
fn default_db() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .map(|h| {
            PathBuf::from(h).join("Library/Application Support/Herdr Agent History/index.sqlite")
        })
        .ok_or_else(|| "HOME is unset; pass --db <path>".into())
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
fn role_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::User => "User",
        EventKind::Assistant => "Assistant",
        EventKind::ToolResult => "Tool",
    }
}
fn print_help() {
    println!("agent-history {VERSION}\n\nUSAGE:\n  agent-history <command> [options]\n\nCOMMANDS:\n  index                 Index Claude and Codex sessions\n  search <query>        Search indexed conversations\n  status                Show index counts and database size\n  preview <agent> <id>  Preview a native session\n\nOPTIONS:\n  --db <path>           SQLite database path\n  --claude-root <path>  Claude projects root\n  --codex-root <path>   Codex sessions root\n  -h, --help            Show help\n  -V, --version         Show version");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_role_filter_without_consuming_query() {
        let args = vec![
            "--role".into(),
            "assistant".into(),
            "multiword".into(),
            "query".into(),
        ];
        let options = Options::parse(&args).unwrap();
        assert_eq!(options.role, Some(EventKind::Assistant));
        assert_eq!(options.query, vec!["multiword", "query"]);
    }

    #[test]
    fn rejects_unknown_role() {
        let args = vec!["--role".into(), "tools".into(), "query".into()];
        assert!(Options::parse(&args).is_err());
    }
}
