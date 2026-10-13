//! The `apply_patch` envelope used by Codex and OpenCode:
//!
//! ```text
//! *** Begin Patch
//! *** Add File: path        then "+" lines
//! *** Delete File: path
//! *** Update File: path
//! *** Move to: new/path     (optional, right after Update File)
//! @@ context                then " ", "-", "+" lines
//! *** End of File           (optional)
//! *** End Patch
//! ```

use crate::model::{FileChange, FileOp};
use crate::util::diff_stat;

/// Per-file changes of a patch. Lenient: unknown lines are kept in the current file's body,
/// a missing `*** Begin Patch` / `*** End Patch` is fine, and garbage yields an empty list.
pub fn parse(patch: &str) -> Vec<FileChange> {
    let mut out: Vec<FileChange> = Vec::new();
    let mut body: Vec<&str> = Vec::new();
    for line in patch.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        let header = |prefix: &str| line.strip_prefix(prefix).map(|p| p.trim().to_string());
        let op = if let Some(p) = header("*** Add File:") {
            Some((FileOp::Add, p))
        } else if let Some(p) = header("*** Update File:") {
            Some((FileOp::Update, p))
        } else {
            header("*** Delete File:").map(|p| (FileOp::Delete, p))
        };
        if let Some((op, path)) = op {
            finish(&mut out, &mut body);
            out.push(FileChange::new(path, op));
            continue;
        }
        if let (Some(to), Some(last)) = (header("*** Move to:"), out.last_mut()) {
            last.op = FileOp::Move;
            last.move_to = Some(to);
            continue;
        }
        if line.trim_end() == "*** End Patch" {
            break;
        }
        if line.starts_with("*** ") {
            continue; // Begin Patch, End of File, Environment ID
        }
        if !out.is_empty() {
            body.push(line);
        }
    }
    finish(&mut out, &mut body);
    out
}

fn finish(out: &mut [FileChange], body: &mut Vec<&str>) {
    let Some(last) = out.last_mut() else { return };
    if last.op != FileOp::Delete {
        let diff = body.join("\n");
        let (adds, dels) = diff_stat(&diff);
        (last.additions, last.deletions) = (Some(adds), Some(dels));
        last.diff = (!body.is_empty()).then_some(diff);
    }
    body.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_operation() {
        let p = "*** Begin Patch\n*** Add File: a.txt\n+one\n+two\n*** Update File: b.rs\n*** Move to: c.rs\n@@ fn main\n-old\n+new\n context\n*** End of File\n*** Delete File: d.md\n*** End Patch\n";
        let files = parse(p);
        assert_eq!(files.len(), 3);
        assert_eq!(
            (files[0].op, files[0].additions, files[0].deletions),
            (FileOp::Add, Some(2), Some(0))
        );
        assert_eq!(files[0].diff.as_deref(), Some("+one\n+two"));
        assert_eq!(
            (files[1].op, files[1].move_to.as_deref()),
            (FileOp::Move, Some("c.rs"))
        );
        assert_eq!((files[1].additions, files[1].deletions), (Some(1), Some(1)));
        assert_eq!(files[1].path, "b.rs");
        assert_eq!(
            (files[2].op, files[2].diff.as_deref()),
            (FileOp::Delete, None)
        );
    }

    #[test]
    fn stops_at_end_marker() {
        let files = parse("*** Begin Patch\n*** Add File: a\n+x\n*** End Patch\ntrailing prose\n");
        assert_eq!(files[0].diff.as_deref(), Some("+x"));
    }

    #[test]
    fn garbage_is_empty() {
        assert!(parse("not a patch\n+x").is_empty());
        assert!(parse("").is_empty());
    }
}
