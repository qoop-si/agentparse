//! Finding sessions on disk without parsing them.

use crate::{Agent, Source, Warning, codex, opencode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::io::BufRead;
use std::path::{Path, PathBuf};

/// Where each product keeps its sessions. `None` skips that product.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Roots {
    /// Claude config dir (`~/.claude`); sessions are under `projects/`.
    pub claude: Option<PathBuf>,
    /// Codex home (`~/.codex`); rollouts are under `sessions/` and `archived_sessions/`.
    pub codex: Option<PathBuf>,
    /// OpenCode data dir (`~/.local/share/opencode`) or one database file.
    pub opencode: Option<PathBuf>,
    /// Gemini dir (`~/.gemini`); chats are under `tmp/<project>/chats/`.
    pub gemini: Option<PathBuf>,
}

impl Roots {
    /// The products' default locations, honoring the overrides each product itself reads:
    /// `CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `OPENCODE_DB` / `XDG_DATA_HOME`, `GEMINI_CLI_HOME`.
    pub fn from_env() -> Roots {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = var("HOME").or_else(|| var("USERPROFILE"));
        let in_home = |rel: &str| home.as_ref().map(|h| h.join(rel));
        let opencode_dir = var("XDG_DATA_HOME")
            .map(|d| d.join("opencode"))
            .or_else(|| in_home(".local/share/opencode"));
        let opencode = match var("OPENCODE_DB") {
            Some(db) if db.is_absolute() => Some(db),
            Some(name) => opencode_dir.map(|d| d.join(name)),
            None => opencode_dir,
        };
        Roots {
            claude: var("CLAUDE_CONFIG_DIR").or_else(|| in_home(".claude")),
            codex: var("CODEX_HOME").or_else(|| in_home(".codex")),
            opencode,
            gemini: var("GEMINI_CLI_HOME")
                .map(|h| h.join(".gemini"))
                .or_else(|| in_home(".gemini")),
        }
    }
}

/// A session found on disk, not parsed yet. `(agent, source)` identifies it; `id` can repeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub agent: Agent,
    pub id: String,
    pub source: Source,
    /// Parent session when the file layout records it (Claude, Gemini, OpenCode). Codex child
    /// threads name their parent only inside the file: see `Session.parent` after parsing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Last write, epoch ms (file mtime, or the database's own update time).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
}

/// List sessions under the roots. A missing or unreadable root becomes a warning, never a failure.
pub fn discover(roots: &Roots) -> (Vec<SessionRef>, Vec<Warning>) {
    let mut found = Found::default();
    if let Some(root) = &roots.claude {
        claude(&root.join("projects"), &mut found);
    }
    if let Some(root) = &roots.codex {
        for dir in ["sessions", "archived_sessions"] {
            for path in found.walk(&root.join(dir), 4) {
                if let Some(id) = codex::thread_id(&file_name(&path)) {
                    found.push(Agent::Codex, id.to_string(), &path, None);
                }
            }
        }
    }
    if let Some(root) = &roots.opencode {
        opencode_root(root, &mut found);
    }
    if let Some(root) = &roots.gemini {
        gemini(&root.join("tmp"), &mut found);
    }
    (found.refs, found.warnings)
}

#[derive(Default)]
struct Found {
    refs: Vec<SessionRef>,
    warnings: Vec<Warning>,
}

impl Found {
    fn push(&mut self, agent: Agent, id: String, path: &Path, parent_id: Option<String>) {
        let source = Source::File {
            path: path.to_path_buf(),
        };
        let updated_at = mtime_ms(path);
        self.refs.push(SessionRef {
            agent,
            id,
            source,
            parent_id,
            updated_at,
        });
    }

    fn warn(&mut self, message: String) {
        self.warnings.push(Warning {
            line: None,
            message,
        });
    }

    /// Sorted entries of a directory. A missing directory is empty; any other read error
    /// becomes a warning, so an unreadable history never looks like an empty one.
    fn entries(&mut self, dir: &Path) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(rd) => {
                let mut paths = Vec::new();
                for entry in rd {
                    match entry {
                        Ok(entry) => paths.push(entry.path()),
                        Err(e) => self.warn(format!("{}: {e}", dir.display())),
                    }
                }
                paths
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                self.warn(format!("{}: {e}", dir.display()));
                Vec::new()
            }
        };
        out.sort();
        out
    }

    fn dirs(&mut self, dir: &Path) -> Vec<PathBuf> {
        self.entries(dir)
            .into_iter()
            .filter(|p| p.is_dir())
            .collect()
    }

    fn files(&mut self, dir: &Path) -> Vec<PathBuf> {
        self.entries(dir)
            .into_iter()
            .filter(|p| p.is_file())
            .collect()
    }

    /// Every file under `dir`, at most `depth` directories deep.
    fn walk(&mut self, dir: &Path, depth: usize) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for path in self.entries(dir) {
            if !path.is_dir() {
                out.push(path);
            } else if depth > 0 {
                out.extend(self.walk(&path, depth - 1));
            }
        }
        out
    }
}

/// `projects/<dir>/<id>.jsonl` and `projects/<dir>/<id>/subagents/**/agent-*.jsonl`.
fn claude(projects: &Path, found: &mut Found) {
    for project in found.dirs(projects) {
        let (sessions, files): (Vec<PathBuf>, Vec<PathBuf>) = found
            .entries(&project)
            .into_iter()
            .partition(|p| p.is_dir());
        for path in files {
            if let Some(id) = file_name(&path).strip_suffix(".jsonl") {
                found.push(Agent::Claude, id.to_string(), &path, None);
            }
        }
        for session in sessions {
            let parent = file_name(&session);
            for path in found.walk(&session.join("subagents"), 3) {
                let name = file_name(&path);
                let Some(agent_id) = name
                    .strip_suffix(".jsonl")
                    .and_then(|s| s.strip_prefix("agent-"))
                else {
                    continue;
                };
                let meta = std::fs::read(path.with_extension("meta.json")).ok();
                let nested = meta
                    .and_then(|m| serde_json::from_slice::<Value>(&m).ok())
                    .and_then(|m| m.get("parentAgentId")?.as_str().map(str::to_string));
                found.push(
                    Agent::Claude,
                    agent_id.to_string(),
                    &path,
                    Some(nested.unwrap_or(parent.clone())),
                );
            }
        }
    }
}

/// Every `opencode*.db` in the data dir (or the one database given), then legacy JSON storage
/// sessions that no database already holds.
fn opencode_root(root: &Path, found: &mut Found) {
    let dbs: Vec<PathBuf> = if root.is_file() {
        vec![root.to_path_buf()]
    } else {
        found
            .files(root)
            .into_iter()
            .filter(|p| {
                let name = file_name(p);
                name.starts_with("opencode") && name.ends_with(".db")
            })
            .collect()
    };
    let mut seen = HashSet::new();
    for db in dbs {
        match opencode::list(&db) {
            Ok(refs) => {
                seen.extend(refs.iter().map(|r| r.id.clone()));
                found.refs.extend(refs);
            }
            Err(e) => found.warn(format!("{}: {e}", db.display())),
        }
    }
    let storage = if root.is_file() {
        root.with_file_name("storage")
    } else {
        root.join("storage")
    };
    for project in found.dirs(&storage.join("session")) {
        for path in found.files(&project) {
            let name = file_name(&path);
            let Some(id) = name.strip_suffix(".json") else {
                continue;
            };
            if seen.insert(id.to_string()) {
                found.push(Agent::Opencode, id.to_string(), &path, None);
            }
        }
    }
}

/// `tmp/<project>/chats/session-*.json[l]` and subagents in `chats/<parent session id>/*.jsonl`.
/// A G1 `.json` resumed by a newer CLI leaves a `.jsonl` twin: the `.jsonl` wins.
fn gemini(tmp: &Path, found: &mut Found) {
    for project in found.dirs(tmp) {
        let chats = project.join("chats");
        let mut chat_files = found.files(&chats);
        chat_files.retain(|p| {
            let ext = p.extension().and_then(|e| e.to_str());
            ext == Some("jsonl")
                || ext == Some("json") && !chats.join(format!("{}l", file_name(p))).exists()
        });
        for path in chat_files {
            let id = gemini_id(&path);
            found.push(Agent::Gemini, id, &path, None);
        }
        for parent in found.dirs(&chats) {
            let parent_id = file_name(&parent);
            for path in found
                .files(&parent)
                .into_iter()
                .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            {
                let id = gemini_id(&path);
                found.push(Agent::Gemini, id, &path, Some(parent_id.clone()));
            }
        }
    }
}

/// The header's `sessionId` of a JSONL chat, else the file stem.
fn gemini_id(path: &Path) -> String {
    let header = std::fs::File::open(path).ok().and_then(|f| {
        let mut line = String::new();
        std::io::BufReader::new(f).read_line(&mut line).ok()?;
        let v: Value = serde_json::from_str(&line).ok()?;
        v.get("sessionId")?.as_str().map(str::to_string)
    });
    header.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string()
    })
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

fn mtime_ms(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since.as_millis()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn finds_every_layout() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        touch(&r.join("claude/projects/-work/s1.jsonl"), "{}");
        touch(
            &r.join("claude/projects/-work/s1/subagents/agent-a1.jsonl"),
            "{}",
        );
        touch(
            &r.join("claude/projects/-work/s1/subagents/agent-a2.jsonl"),
            "{}",
        );
        touch(
            &r.join("claude/projects/-work/s1/subagents/agent-a2.meta.json"),
            r#"{"parentAgentId":"a1"}"#,
        );
        touch(
            &r.join("claude/projects/-work/s1/subagents/workflows/w1/agent-x-1f.jsonl"),
            "{}",
        );
        touch(
            &r.join("claude/projects/-work/s1/subagents/workflows/w1/journal.jsonl"),
            "{}",
        );
        let uuid = "019a0000-0000-7000-8000-000000000001";
        touch(
            &r.join(format!(
                "codex/sessions/2026/10/11/rollout-2026-10-11T14-50-06-{uuid}.jsonl"
            )),
            "{}",
        );
        touch(
            &r.join(format!(
                "codex/archived_sessions/rollout-2026-01-01T00-00-00-{uuid}_r2.jsonl.zst"
            )),
            "",
        );
        touch(&r.join("gemini/tmp/p/chats/session-a.json"), "{}");
        touch(
            &r.join("gemini/tmp/p/chats/session-a.jsonl"),
            r#"{"sessionId":"g-1","projectHash":"h"}"#,
        );
        touch(&r.join("gemini/tmp/p/chats/session-b.json"), "{}");
        touch(
            &r.join("gemini/tmp/p/chats/g-1/agent-7.jsonl"),
            r#"{"sessionId":"g-2","projectHash":"h"}"#,
        );
        let roots = Roots {
            claude: Some(r.join("claude")),
            codex: Some(r.join("codex")),
            opencode: Some(r.join("missing-opencode")),
            gemini: Some(r.join("gemini")),
        };
        let (refs, _) = discover(&roots);
        let got: Vec<(Agent, &str, Option<&str>)> = refs
            .iter()
            .map(|s| (s.agent, s.id.as_str(), s.parent_id.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (Agent::Claude, "s1", None),
                (Agent::Claude, "a1", Some("s1")),
                (Agent::Claude, "a2", Some("a1")),
                (Agent::Claude, "x-1f", Some("s1")),
                (Agent::Codex, uuid, None),
                (Agent::Codex, uuid, None),
                (Agent::Gemini, "g-1", None),
                (Agent::Gemini, "session-b", None),
                (Agent::Gemini, "g-2", Some("g-1")),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_dir_is_a_warning() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let locked = t.path().join("projects/-work");
        touch(&locked.join("s1.jsonl"), "{}");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let (refs, warnings) = discover(&Roots {
            claude: Some(t.path().into()),
            ..Default::default()
        });
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(refs.is_empty());
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn missing_roots_are_empty_not_errors() {
        let roots = Roots {
            claude: Some("/nonexistent/x".into()),
            ..Default::default()
        };
        let (refs, warnings) = discover(&roots);
        assert!(refs.is_empty() && warnings.is_empty());
    }
}
