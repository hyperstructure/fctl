//! The diffs of a session's changes, for people to read on its page.
//!
//! fctl-agent reads them with `git diff` ([`parse`]) as it reads the
//! repositories' state, adds those of the files git doesn't track yet
//! ([`untracked`]), keeps them within what a session keeps and seals each
//! with its digest ([`fit`]), and hands them all to its host. The host
//! passes on to SessionDO those its uplink hasn't sent, and SessionDO checks
//! each ([`check`]) before it keeps the latest of every changed file's.
//! `fctl diff` prints them as git would ([`git_patch`]).

use fctl_proto::event::FileDiff;
use sha2::{Digest, Sha256};

/// The most of one file's diff a session keeps: its patch's bytes as JSON
/// has them, escapes included, so that the largest still fits an uplink
/// frame ([`MAX_FRAME_BYTES`]).
pub const MAX_PATCH_BYTES: usize = 512 * 1024;

/// The most of all of its files' diffs a session keeps, counted as
/// [`MAX_PATCH_BYTES`] counts one's.
pub const MAX_SESSION_BYTES: usize = 8 * 1024 * 1024;

/// The most changed files a session keeps the diffs of, in all of its
/// repositories.
pub const MAX_FILES: usize = 2_000;

/// The most bytes of diffs an uplink frame carries (`UplinkUp::Diff`'s
/// `files`, as JSON): well within the message a Durable Object takes.
pub const MAX_FRAME_BYTES: usize = 768 * 1024;

/// The longest path a diff may have, in bytes.
pub const MAX_PATH_BYTES: usize = 4096;

/// How much of a file git looks at for a NUL byte to call it binary.
const BINARY_SNIFF: usize = 8000;

/// What [`FileDiff::status`] may say, as [`fctl_proto::event::FileChange`]'s.
const STATUSES: [&str; 5] = ["added", "modified", "deleted", "renamed", "untracked"];

/// The arguments of the `git diff` whose output [`parse`] reads, before the
/// commit to compare with: paths as they are (no octal escapes for
/// letters beyond ASCII), no colors, no tools of the repository's own, and
/// the prefixes it expects, whatever the repository's configuration says.
pub const GIT_DIFF: [&str; 10] = [
    "-c",
    "core.quotepath=off",
    "diff",
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "-M",
    "-U3",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// The files in `git diff`'s output ([`GIT_DIFF`]), each with its hunks.
/// Their digests are those of their whole part of the output, which
/// [`fit`] seals.
pub fn parse(out: &str) -> Vec<FileDiff> {
    let mut sections = Vec::new();
    let mut start = None;
    let lines: Vec<&str> = out.split('\n').collect();
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("diff --git ") {
            if let Some(s) = start {
                sections.extend(section(&lines[s..i]));
            }
            start = Some(i);
        }
    }
    if let Some(s) = start {
        let mut end = lines.len();
        // The output's last newline leaves an empty line after it.
        while end > s + 1 && lines[end - 1].is_empty() {
            end -= 1;
        }
        sections.extend(section(&lines[s..end]));
    }
    let mut files: Vec<FileDiff> = Vec::with_capacity(sections.len());
    for f in sections {
        // A file whose type changed (a file that became a symbolic link, or
        // back) comes as its deletion, then its addition: one change.
        match files.last_mut() {
            Some(last)
                if last.path == f.path && last.status == "deleted" && f.status == "added" =>
            {
                type_changed(last, f);
            }
            _ => files.push(f),
        }
    }
    files
}

/// `old`, the deletion half of a file whose type changed, with `new`, its
/// addition half: both halves' hunks, and its modes before and after.
fn type_changed(old: &mut FileDiff, new: FileDiff) {
    old.status = "modified".to_owned();
    old.patch = match (old.patch.take(), new.patch) {
        (Some(a), Some(b)) => Some(format!("{a}\n{b}")),
        (a, b) => a.or(b),
    };
    old.binary |= new.binary;
    old.new_mode = new.new_mode;
    let mut hash = Sha256::new();
    hash.update(old.digest.as_bytes());
    hash.update(new.digest.as_bytes());
    old.digest = hex::encode(hash.finalize());
}

/// One file's part of `git diff`'s output: its header, then its hunks.
fn section(lines: &[&str]) -> Option<FileDiff> {
    let (first, rest) = lines.split_first()?;
    let header = first.strip_prefix("diff --git ")?;
    let mut status = "modified";
    let (mut from, mut to, mut minus, mut plus) = (None, None, None, None);
    let (mut old_mode, mut new_mode) = (None, None);
    let mut binary = false;
    let mut hunks = None;
    for (i, line) in rest.iter().enumerate() {
        if line.starts_with("@@") {
            hunks = Some(i);
            break;
        }
        if let Some(m) = line.strip_prefix("old mode ") {
            old_mode = Some(m.trim().to_owned());
        } else if let Some(m) = line.strip_prefix("new mode ") {
            new_mode = Some(m.trim().to_owned());
        } else if let Some(m) = line.strip_prefix("deleted file mode ") {
            status = "deleted";
            old_mode = Some(m.trim().to_owned());
        } else if let Some(m) = line.strip_prefix("new file mode ") {
            status = "added";
            new_mode = Some(m.trim().to_owned());
        } else if let Some(p) = line.strip_prefix("rename from ") {
            from = Some(unquote(p));
            status = "renamed";
        } else if let Some(p) = line.strip_prefix("rename to ") {
            to = Some(unquote(p));
            status = "renamed";
        } else if let Some(p) = line.strip_prefix("--- ") {
            minus = side(p, "a/");
        } else if let Some(p) = line.strip_prefix("+++ ") {
            plus = side(p, "b/");
        } else if line.starts_with("Binary files ") || *line == "GIT binary patch" {
            binary = true;
        }
    }
    // `/dev/null` names no path: a new file's is on the `+++` line, a
    // deleted one's on the `---` line, and only the header has a binary
    // file's or one whose mode alone changed.
    let (a, b) = header_paths(header).unzip();
    let path = to.or(plus).or(minus).or(b).or(a)?;
    let patch = hunks.map(|i| rest[i..].join("\n"));
    Some(FileDiff {
        repo: None,
        path,
        old_path: from.filter(|_| status == "renamed"),
        status: status.to_owned(),
        digest: hex::encode(Sha256::digest(lines.join("\n").as_bytes())),
        patch,
        binary,
        too_large: false,
        old_mode,
        new_mode,
    })
}

/// A path on a `---` or `+++` line: none for `/dev/null`. git ends the
/// line with a tab when the path has a space.
fn side(text: &str, prefix: &str) -> Option<String> {
    let text = text.strip_suffix('\t').unwrap_or(text);
    if text == "/dev/null" {
        return None;
    }
    let path = unquote(text);
    Some(path.strip_prefix(prefix).map(str::to_owned).unwrap_or(path))
}

/// The paths of a `diff --git a/… b/…` line, without their prefixes. git
/// quotes either when it has to; unquoted, they can only be told apart
/// when they are the same, which they are unless the file was renamed,
/// whose header lines name both anyway.
fn header_paths(header: &str) -> Option<(String, String)> {
    let (a, rest) = if header.starts_with('"') {
        let (a, used) = quoted(header)?;
        (a, header.get(used..)?.strip_prefix(' ')?.to_owned())
    } else if let Some(i) = header.find(" \"b/") {
        (header[..i].to_owned(), header[i + 1..].to_owned())
    } else {
        // `a/P b/P`: the same path twice, a space between.
        let half = header.len().checked_sub(1)? / 2;
        let (a, b) = (header.get(..half)?, header.get(half + 1..)?);
        if header.as_bytes().get(half) == Some(&b' ')
            && a.strip_prefix("a/").is_some()
            && a.get(2..) == b.get(2..)
        {
            (a.to_owned(), b.to_owned())
        } else {
            let i = header.find(" b/")?;
            (header[..i].to_owned(), header[i + 1..].to_owned())
        }
    };
    let b = if rest.starts_with('"') {
        quoted(&rest)?.0
    } else {
        rest
    };
    Some((
        a.strip_prefix("a/").unwrap_or(&a).to_owned(),
        b.strip_prefix("b/").unwrap_or(&b).to_owned(),
    ))
}

/// A path as git writes it: in double quotes, with C's escapes, when it
/// holds a quote, a backslash or a control character.
pub fn unquote(text: &str) -> String {
    match quoted(text) {
        Some((s, used)) if used == text.len() => s,
        _ => text.to_owned(),
    }
}

/// The C-quoted string `text` starts with, and how many of its bytes it
/// took; none unless it starts with a quote and the quote closes.
fn quoted(text: &str) -> Option<(String, usize)> {
    let body = text.strip_prefix('"')?;
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), i + 2)),
            b'\\' => {
                let c = *bytes.get(i + 1)?;
                i += 2;
                match c {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'a' => out.push(7),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'v' => out.push(11),
                    b'0'..=b'3' => {
                        let digits = bytes.get(i - 1..i + 2)?;
                        let mut n: u8 = 0;
                        for d in digits {
                            if !(b'0'..=b'7').contains(d) {
                                return None;
                            }
                            n = n.checked_mul(8)?.checked_add(d - b'0')?;
                        }
                        out.push(n);
                        i += 2;
                    }
                    other => out.push(other),
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    None
}

/// The diff of a file git doesn't track yet, which `git diff` leaves out:
/// every line of `bytes`, its content, added. `whole` says whether
/// `bytes` is all of it: a larger file's diff is too large to keep anyway.
pub fn untracked(path: &str, bytes: &[u8], whole: bool) -> FileDiff {
    let mut file = FileDiff {
        repo: None,
        path: path.to_owned(),
        old_path: None,
        status: "untracked".to_owned(),
        digest: String::new(),
        patch: None,
        binary: false,
        too_large: false,
        old_mode: None,
        new_mode: None,
    };
    let mut hash = Sha256::new();
    hash.update(path.as_bytes());
    hash.update([0]);
    hash.update(bytes);
    if !whole {
        hash.update(b"\0more");
    }
    file.digest = hex::encode(hash.finalize());
    if bytes[..bytes.len().min(BINARY_SNIFF)].contains(&0) {
        file.binary = true;
        return file;
    }
    if !whole {
        file.too_large = true;
        return file;
    }
    if bytes.is_empty() {
        return file;
    }
    let text = String::from_utf8_lossy(bytes);
    let ends = text.ends_with('\n');
    let body = text.strip_suffix('\n').unwrap_or(&text);
    let lines: Vec<&str> = body.split('\n').collect();
    let count = if lines.len() == 1 {
        "1".to_owned()
    } else {
        format!("1,{}", lines.len())
    };
    let mut patch = format!("@@ -0,0 +{count} @@");
    for line in &lines {
        patch.push_str("\n+");
        patch.push_str(line);
    }
    if !ends {
        patch.push_str("\n\\ No newline at end of file");
    }
    file.patch = Some(patch);
    file
}

/// A file whose diff is too large to read, let alone keep: `what` says
/// what its diff is, in some way that changes as it does.
pub fn too_large(path: &str, what: &str) -> FileDiff {
    let mut hash = Sha256::new();
    hash.update(path.as_bytes());
    hash.update([0]);
    hash.update(what.as_bytes());
    FileDiff {
        repo: None,
        path: path.to_owned(),
        old_path: None,
        status: "modified".to_owned(),
        digest: hex::encode(hash.finalize()),
        patch: None,
        binary: false,
        too_large: true,
        old_mode: None,
        new_mode: None,
    }
}

/// The bytes a patch takes as JSON has it, which is what the limits count.
pub fn json_bytes(patch: &str) -> usize {
    serde_json::to_string(patch).map_or(usize::MAX, |s| s.len())
}

/// Keeps `files` within what a session keeps of them: the first
/// [`MAX_FILES`], each patch within [`MAX_PATCH_BYTES`], and, the smallest
/// first, as many as fit in [`MAX_SESSION_BYTES`]; the others are too
/// large, without their patches. Then seals each one's digest, which says
/// whether it was too large too, so that SessionDO gets its patch once it
/// fits again. Call it once, with the digests [`parse`] and [`untracked`]
/// gave them.
pub fn fit(files: &mut Vec<FileDiff>) {
    files.truncate(MAX_FILES);
    let mut sizes: Vec<(usize, usize)> = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.patch.as_deref().map_or(0, json_bytes), i))
        .collect();
    sizes.sort_unstable();
    let mut left = MAX_SESSION_BYTES;
    for (size, i) in sizes {
        let Some(f) = files.get_mut(i) else { continue };
        if size > MAX_PATCH_BYTES || size > left {
            f.patch = None;
            f.too_large = true;
        } else {
            left -= size;
        }
    }
    for f in files.iter_mut() {
        let mut hash = Sha256::new();
        hash.update(f.digest.as_bytes());
        hash.update(f.status.as_bytes());
        if f.too_large {
            hash.update(b"\0too large");
        }
        f.digest = hex::encode(&hash.finalize()[..8]);
    }
}

/// Why SessionDO doesn't keep a diff an uplink sent: what runs in a
/// sandbox can pose as its agent and send anything.
pub fn check(f: &FileDiff) -> Result<(), &'static str> {
    let path_ok = |p: &str| !p.is_empty() && p.len() <= MAX_PATH_BYTES && !p.contains('\0');
    if !path_ok(&f.path) || f.old_path.as_deref().is_some_and(|p| !path_ok(p)) {
        return Err("a path that is empty, too long or holds a NUL");
    }
    if f.repo.as_deref().is_some_and(|r| !repo_name(r)) {
        return Err("a repository that isn't owner/name");
    }
    if !STATUSES.contains(&f.status.as_str()) {
        return Err("an unknown status");
    }
    if f.digest.is_empty()
        || f.digest.len() > 64
        || !f.digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("a digest that isn't hexadecimal");
    }
    if f.patch
        .as_deref()
        .is_some_and(|p| p.len() > MAX_PATCH_BYTES)
    {
        return Err("a patch larger than a session keeps");
    }
    let mode_ok =
        |m: &str| !m.is_empty() && m.len() <= 6 && m.bytes().all(|b| (b'0'..=b'7').contains(&b));
    if [&f.old_mode, &f.new_mode]
        .into_iter()
        .flatten()
        .any(|m| !mode_ok(m))
    {
        return Err("a mode that isn't octal");
    }
    Ok(())
}

/// `owner/name`, as GitHub has them.
fn repo_name(r: &str) -> bool {
    let ok = |s: &str| {
        !s.is_empty()
            && s.len() <= 100
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    };
    matches!(r.split_once('/'), Some((owner, name)) if ok(owner) && ok(name))
}

/// The file's diff as git prints it, for `fctl diff`: its header, then its
/// hunks. A binary file's says so, as git's does; one too large for the
/// session to keep has its header alone.
pub fn git_patch(f: &FileDiff) -> String {
    // A type change, as git writes it: the file's deletion, then its addition.
    if let (Some(o), Some(n)) = (&f.old_mode, &f.new_mode)
        && (o == SYMLINK) != (n == SYMLINK)
    {
        let (gone, came) = split_type_change(f.patch.as_deref());
        let half = |status: &str, patch: Option<String>, old_mode, new_mode| FileDiff {
            status: status.to_owned(),
            patch,
            old_mode,
            new_mode,
            ..f.clone()
        };
        return git_patch(&half("deleted", gone, Some(o.clone()), None))
            + &git_patch(&half("added", came, None, Some(n.clone())));
    }
    let old = f.old_path.as_deref().unwrap_or(&f.path);
    let q = |prefix: &str, p: &str| quote(&format!("{prefix}{p}"));
    let mut out = format!("diff --git {} {}\n", q("a/", old), q("b/", &f.path));
    let added = matches!(f.status.as_str(), "added" | "untracked");
    let deleted = f.status == "deleted";
    if let (Some(o), Some(n), false, false) = (&f.old_mode, &f.new_mode, added, deleted) {
        out.push_str(&format!("old mode {o}\nnew mode {n}\n"));
    } else if added {
        out.push_str(&format!(
            "new file mode {}\n",
            f.new_mode.as_deref().unwrap_or("100644")
        ));
    } else if deleted {
        out.push_str(&format!(
            "deleted file mode {}\n",
            f.old_mode.as_deref().unwrap_or("100644")
        ));
    }
    if let Some(from) = &f.old_path {
        out.push_str(&format!(
            "rename from {}\nrename to {}\n",
            quote(from),
            quote(&f.path)
        ));
    }
    let minus = if added {
        "/dev/null".to_owned()
    } else {
        q("a/", old)
    };
    let plus = if deleted {
        "/dev/null".to_owned()
    } else {
        q("b/", &f.path)
    };
    if f.binary {
        out.push_str(&format!("Binary files {minus} and {plus} differ\n"));
    } else if let Some(patch) = &f.patch {
        out.push_str(&format!("--- {minus}\n+++ {plus}\n{patch}\n"));
    }
    out
}

/// A symbolic link's mode.
const SYMLINK: &str = "120000";

/// A type change's hunks, as [`parse`] joined them: those that only take
/// lines out (`+0,0`), and those that only add them (`-0,0`).
fn split_type_change(patch: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(patch) = patch else {
        return (None, None);
    };
    let (mut gone, mut came): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    let mut adding = false;
    for line in patch.split('\n') {
        if line.starts_with("@@") {
            adding = line.starts_with("@@ -0,0 ");
        }
        if adding {
            came.push(line);
        } else {
            gone.push(line);
        }
    }
    let joined = |v: Vec<&str>| (!v.is_empty()).then(|| v.join("\n"));
    (joined(gone), joined(came))
}

/// A path as git writes it: quoted, with C's escapes, when it has to be.
fn quote(path: &str) -> String {
    if !path
        .chars()
        .any(|c| c == '"' || c == '\\' || c.is_control())
    {
        return path.to_owned();
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("\\{b:03o}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A file `git diff --numstat -z -M` lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Numstat {
    /// Lines added and taken out; none for a binary file.
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub path: String,
    /// The path a renamed file had.
    pub old_path: Option<String>,
}

/// The files of `git diff --numstat -z -M`'s output: `added\tdeleted\tpath`,
/// or for a rename `added\tdeleted\t` then the old path and the new one,
/// each ended by a NUL.
pub fn numstat(out: &str) -> Vec<Numstat> {
    let mut files = Vec::new();
    let mut fields = out.split('\0');
    while let Some(entry) = fields.next() {
        let mut parts = entry.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let (path, old_path) = if path.is_empty() {
            let (Some(old), Some(new)) = (fields.next(), fields.next()) else {
                break;
            };
            (new.to_owned(), Some(old.to_owned()))
        } else {
            (path.to_owned(), None)
        };
        files.push(Numstat {
            additions: a.parse().ok(),
            deletions: d.parse().ok(),
            path,
            old_path,
        });
    }
    files
}

/// The files of `git diff --name-status -z -M`'s output, with the status
/// each has as a [`fctl_proto::event::FileChange`]'s: a letter, then the
/// path, or for a rename or a copy the old path and the new one, each
/// ended by a NUL.
pub fn name_status(out: &str) -> Vec<(String, &'static str)> {
    let mut files = Vec::new();
    let mut fields = out.split('\0');
    while let Some(code) = fields.next() {
        let Some(letter) = code.chars().next() else {
            continue;
        };
        let two = matches!(letter, 'R' | 'C');
        let first = fields.next();
        let path = if two { fields.next() } else { first };
        let Some(path) = path else { break };
        let status = match letter {
            'A' | 'C' => "added",
            'D' => "deleted",
            'R' => "renamed",
            _ => "modified",
        };
        files.push((path.to_owned(), status));
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What git printed for a few changes, as in `git diff` with [`GIT_DIFF`]
    /// (a binary file, a deleted file with a quote in its name, a mode
    /// change, a path with a space, a rename with an edit).
    const OUTPUT: &str = "diff --git a/bin.dat b/bin.dat
index 731e575..237d8d7 100644
Binary files a/bin.dat and b/bin.dat differ
diff --git \"a/quo\\\"te.txt\" \"b/quo\\\"te.txt\"
deleted file mode 100644
index bca70f3..0000000
--- \"a/quo\\\"te.txt\"
+++ /dev/null
@@ -1 +0,0 @@
-q
diff --git a/run.sh b/run.sh
old mode 100644
new mode 100755
diff --git a/sp ace.txt b/sp ace.txt
index 587be6b..b77b4eb 100644
--- a/sp ace.txt\t
+++ b/sp ace.txt\t
@@ -1 +1,2 @@
 x
+y
diff --git a/src/old.rs b/src/new.rs
similarity index 83%
rename from src/old.rs
rename to src/new.rs
index c521009..e3b73eb 100644
--- a/src/old.rs
+++ b/src/new.rs
@@ -3,3 +3,4 @@ fn b() {}
 fn c() {}
 fn d() {}
 fn e() {}
+fn z() {}
";

    #[test]
    fn reads_each_file_of_git_s_output() {
        let files = parse(OUTPUT);
        let seen: Vec<(&str, Option<&str>, &str, bool)> = files
            .iter()
            .map(|f| {
                (
                    f.path.as_str(),
                    f.old_path.as_deref(),
                    f.status.as_str(),
                    f.binary,
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("bin.dat", None, "modified", true),
                ("quo\"te.txt", None, "deleted", false),
                ("run.sh", None, "modified", false),
                ("sp ace.txt", None, "modified", false),
                ("src/new.rs", Some("src/old.rs"), "renamed", false),
            ]
        );
        assert_eq!(files[0].patch, None);
        assert_eq!(files[1].patch.as_deref(), Some("@@ -1 +0,0 @@\n-q"));
        // A mode changed alone: no lines.
        assert_eq!(files[2].patch, None);
        assert_eq!(files[2].old_mode.as_deref(), Some("100644"));
        assert_eq!(files[2].new_mode.as_deref(), Some("100755"));
        assert_eq!(files[3].patch.as_deref(), Some("@@ -1 +1,2 @@\n x\n+y"));
        // The last file's hunks end where the output does.
        assert_eq!(
            files[4].patch.as_deref(),
            Some("@@ -3,3 +3,4 @@ fn b() {}\n fn c() {}\n fn d() {}\n fn e() {}\n+fn z() {}")
        );
        // Each file's digest is its own part's.
        let digests: std::collections::HashSet<_> = files.iter().map(|f| &f.digest).collect();
        assert_eq!(digests.len(), files.len());
        assert!(parse("").is_empty());
    }

    #[test]
    fn a_binary_file_s_digest_follows_its_content() {
        let before = parse(OUTPUT);
        let after = parse(&OUTPUT.replace("731e575..237d8d7", "731e575..6adc4fc"));
        assert_ne!(before[0].digest, after[0].digest);
        assert_eq!(before[1..], after[1..]);
    }

    #[test]
    fn a_file_that_became_a_symbolic_link_is_one_change() {
        // What git printed for `x`, a file of two lines that became a link.
        let out = "diff --git a/x b/x
deleted file mode 100644
index 814f4a4..0000000
--- a/x
+++ /dev/null
@@ -1,2 +0,0 @@
-one
-two
diff --git a/x b/x
new file mode 120000
index 0000000..1de5659
--- /dev/null
+++ b/x
@@ -0,0 +1 @@
+target
\\ No newline at end of file
";
        let files = parse(out);
        assert_eq!(files.len(), 1);
        let x = &files[0];
        assert_eq!((x.path.as_str(), x.status.as_str()), ("x", "modified"));
        assert_eq!(
            (x.old_mode.as_deref(), x.new_mode.as_deref()),
            (Some("100644"), Some("120000"))
        );
        assert_eq!(
            x.patch.as_deref(),
            Some(
                "@@ -1,2 +0,0 @@\n-one\n-two\n@@ -0,0 +1 @@\n+target\n\\ No newline at end of file"
            )
        );
        // Either half changing changes its digest.
        let other = parse(&out.replace("+target", "+elsewhere"));
        assert_ne!(other[0].digest, x.digest);
        // Printed as git does, the deletion then the addition, but for the
        // blobs' ids, which aren't kept.
        let without_index: String = out
            .lines()
            .filter(|l| !l.starts_with("index "))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(git_patch(x), without_index);
    }

    #[test]
    fn hunks_keep_lines_without_a_newline_and_carriage_returns() {
        let out = "diff --git a/n.txt b/n.txt
index 20cbb4d..87cb42f 100644
--- a/n.txt
+++ b/n.txt
@@ -1 +1,2 @@
-no newline
\\ No newline at end of file
+no newline\r
+more
\\ No newline at end of file
";
        let files = parse(out);
        assert_eq!(
            files[0].patch.as_deref(),
            Some(
                "@@ -1 +1,2 @@\n-no newline\n\\ No newline at end of file\n+no newline\r\n+more\n\\ No newline at end of file"
            )
        );
    }

    #[test]
    fn a_line_that_looks_like_a_header_inside_a_hunk_is_content() {
        // Content lines start with a space, + or -: never with "diff --git".
        let out = "diff --git a/a.md b/a.md
index 1..2 100644
--- a/a.md
+++ b/a.md
@@ -1 +1,2 @@
 diff --git a/x b/x
+--- a/y
";
        let files = parse(out);
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].patch.as_deref(),
            Some("@@ -1 +1,2 @@\n diff --git a/x b/x\n+--- a/y")
        );
    }

    #[test]
    fn paths_come_from_the_header_when_nothing_else_names_them() {
        assert_eq!(
            header_paths("a/a b/c b/a b/c"),
            Some(("a b/c".to_owned(), "a b/c".to_owned()))
        );
        assert_eq!(
            header_paths("\"a/t\\tab\" \"b/t\\tab\""),
            Some(("t\tab".to_owned(), "t\tab".to_owned()))
        );
        assert_eq!(
            header_paths("a/x \"b/y\\\"\""),
            Some(("x".to_owned(), "y\"".to_owned()))
        );
        // Renamed, unquoted: the first " b/" splits them.
        assert_eq!(
            header_paths("a/old b/new"),
            Some(("old".to_owned(), "new".to_owned()))
        );
    }

    #[test]
    fn unquotes_c_escapes() {
        assert_eq!(unquote("plain"), "plain");
        assert_eq!(unquote("\"a\\\"b\\\\c\""), "a\"b\\c");
        // Octal bytes, as git writes what isn't ASCII without core.quotepath=off.
        assert_eq!(unquote("\"caf\\303\\251\""), "café");
        // Not closed: as it is.
        assert_eq!(unquote("\"open"), "\"open");
        assert_eq!(quote("café"), "café");
        assert_eq!(quote("a\"b\tc"), "\"a\\\"b\\tc\"");
        assert_eq!(unquote(&quote("x\u{1}y")), "x\u{1}y");
    }

    #[test]
    fn untracked_files_are_all_added() {
        let f = untracked("notes.md", b"# Notes\n\nOne\n", true);
        assert_eq!(f.status, "untracked");
        assert_eq!(
            f.patch.as_deref(),
            Some("@@ -0,0 +1,3 @@\n+# Notes\n+\n+One")
        );
        let one = untracked("x", b"x", true);
        assert_eq!(
            one.patch.as_deref(),
            Some("@@ -0,0 +1 @@\n+x\n\\ No newline at end of file")
        );
        assert_eq!(untracked("empty", b"", true).patch, None);
        let bin = untracked("a.png", b"\x89PNG\0\0", true);
        assert!(bin.binary && bin.patch.is_none());
        let big = untracked("big.log", b"partial", false);
        assert!(big.too_large && big.patch.is_none());
        assert_ne!(
            untracked("x", b"a\n", true).digest,
            untracked("x", b"b\n", true).digest
        );
        assert_ne!(
            untracked("x", b"a\n", true).digest,
            untracked("y", b"a\n", true).digest
        );
    }

    #[test]
    fn fit_keeps_what_a_session_keeps_the_smallest_first() {
        let file = |path: &str, bytes: usize| {
            let mut f = untracked(path, b"", true);
            f.patch = Some("x".repeat(bytes));
            f
        };
        let mut files = vec![
            file("huge", MAX_PATCH_BYTES + 1),
            file("a", MAX_PATCH_BYTES - 2),
            file("small", 10),
        ];
        // Fifteen more as large as a can be: sixteen of them pass the budget.
        for i in 0..15 {
            files.push(file(&format!("b{i:02}"), MAX_PATCH_BYTES - 2));
        }
        let before: Vec<String> = files.iter().map(|f| f.digest.clone()).collect();
        fit(&mut files);
        let large: Vec<&str> = files
            .iter()
            .filter(|f| f.too_large)
            .map(|f| f.path.as_str())
            .collect();
        // The huge one, and the last of the large ones that came to more than the session keeps.
        assert_eq!(large, ["huge", "b14"]);
        assert!(
            files
                .iter()
                .filter(|f| f.too_large)
                .all(|f| f.patch.is_none())
        );
        assert!(files[2].patch.is_some());
        // Sealed: short, and different from what they were.
        assert!(files.iter().all(|f| f.digest.len() == 16));
        assert!(files.iter().zip(&before).all(|(f, b)| &f.digest != b));
    }

    #[test]
    fn a_file_too_large_once_gets_another_digest_than_when_it_fits() {
        let mut big = vec![untracked("x", b"a\n", true)];
        let mut small = big.clone();
        big[0].patch = Some("y".repeat(MAX_PATCH_BYTES + 10));
        // The same content's digest, too large or not.
        fit(&mut big);
        fit(&mut small);
        assert!(big[0].too_large && !small[0].too_large);
        assert_ne!(big[0].digest, small[0].digest);
    }

    #[test]
    fn escapes_count_toward_the_limits() {
        assert_eq!(json_bytes("ab"), 4);
        assert_eq!(json_bytes("\u{1}"), 8);
        let mut files = vec![untracked("x", b"", true)];
        files[0].patch = Some("\u{1}".repeat(MAX_PATCH_BYTES / 4));
        fit(&mut files);
        assert!(files[0].too_large);
    }

    #[test]
    fn session_do_refuses_what_no_agent_sends() {
        let mut ok = untracked("src/main.rs", b"fn main() {}\n", true);
        let mut files = vec![ok.clone()];
        fit(&mut files);
        ok = files.remove(0);
        assert_eq!(check(&ok), Ok(()));
        let with = |f: &dyn Fn(&mut FileDiff)| {
            let mut d = ok.clone();
            f(&mut d);
            check(&d)
        };
        assert!(with(&|d| d.path = String::new()).is_err());
        assert!(with(&|d| d.path = "a\0b".into()).is_err());
        assert!(with(&|d| d.path = "x".repeat(MAX_PATH_BYTES + 1)).is_err());
        assert!(with(&|d| d.old_path = Some(String::new())).is_err());
        assert!(with(&|d| d.repo = Some("acme".into())).is_err());
        assert!(with(&|d| d.repo = Some("acme/web app".into())).is_err());
        assert_eq!(with(&|d| d.repo = Some("acme/web.app".into())), Ok(()));
        assert!(with(&|d| d.status = "exploded".into()).is_err());
        assert!(with(&|d| d.digest = "xyz".into()).is_err());
        assert!(with(&|d| d.digest = String::new()).is_err());
        assert!(with(&|d| d.patch = Some("x".repeat(MAX_PATCH_BYTES + 1))).is_err());
        assert!(with(&|d| d.new_mode = Some("100789".into())).is_err());
        assert_eq!(with(&|d| d.new_mode = Some("100755".into())), Ok(()));
    }

    #[test]
    fn prints_each_file_as_git_does() {
        let files = parse(OUTPUT);
        let printed: String = files.iter().map(git_patch).collect();
        // What git apply needs of them: the headers, the --- and +++ lines, the hunks.
        assert!(printed.contains("diff --git \"a/quo\\\"te.txt\" \"b/quo\\\"te.txt\"\ndeleted file mode 100644\n--- \"a/quo\\\"te.txt\"\n+++ /dev/null\n@@ -1 +0,0 @@\n-q\n"));
        assert!(
            printed.contains("diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n")
        );
        assert!(printed.contains("diff --git a/src/old.rs b/src/new.rs\nrename from src/old.rs\nrename to src/new.rs\n--- a/src/old.rs\n+++ b/src/new.rs\n@@ -3,3 +3,4 @@"));
        assert!(printed.contains("Binary files a/bin.dat and b/bin.dat differ\n"));
        let new = untracked("notes.md", b"hi\n", true);
        assert_eq!(
            git_patch(&new),
            "diff --git a/notes.md b/notes.md\nnew file mode 100644\n--- /dev/null\n+++ b/notes.md\n@@ -0,0 +1 @@\n+hi\n"
        );
        // Reading it back gives the same files.
        let again = parse(&printed);
        let paths = |fs: &[FileDiff]| fs.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&again), paths(&files));
        assert_eq!(
            again.iter().map(|f| &f.patch).collect::<Vec<_>>(),
            files.iter().map(|f| &f.patch).collect::<Vec<_>>()
        );
    }

    #[test]
    fn reads_numstat_and_name_status_with_nuls() {
        let out = "-\t-\tbin.dat\x000\t1\tquo\"te.txt\x001\t0\tsp ace.txt\x001\t0\t\x00src/old.rs\x00src/new.rs\x00";
        assert_eq!(
            numstat(out),
            [
                Numstat {
                    additions: None,
                    deletions: None,
                    path: "bin.dat".into(),
                    old_path: None
                },
                Numstat {
                    additions: Some(0),
                    deletions: Some(1),
                    path: "quo\"te.txt".into(),
                    old_path: None
                },
                Numstat {
                    additions: Some(1),
                    deletions: Some(0),
                    path: "sp ace.txt".into(),
                    old_path: None
                },
                Numstat {
                    additions: Some(1),
                    deletions: Some(0),
                    path: "src/new.rs".into(),
                    old_path: Some("src/old.rs".into())
                },
            ]
        );
        assert_eq!(
            name_status(
                "M\x00bin.dat\x00D\x00quo\"te.txt\x00R083\x00src/old.rs\x00src/new.rs\x00A\x00n.md\x00"
            ),
            [
                ("bin.dat".to_owned(), "modified"),
                ("quo\"te.txt".to_owned(), "deleted"),
                ("src/new.rs".to_owned(), "renamed"),
                ("n.md".to_owned(), "added"),
            ]
        );
        assert!(numstat("").is_empty());
        assert!(name_status("").is_empty());
    }
}
