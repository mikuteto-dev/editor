use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use super::git;

pub fn branch(dir: &Path) -> Option<String> {
    // symbolic-ref also names the branch of a repository without commits.
    git(dir, &["symbolic-ref", "--short", "HEAD"], &[0])
        .or_else(|| git(dir, &["rev-parse", "--short", "HEAD"], &[0]))
        .map(|s| s.trim().to_string())
}

pub struct Change {
    pub path: PathBuf,
    pub status: char,
    pub staged: bool,
    pub unstaged: bool,
}

impl Change {
    pub fn untracked(&self) -> bool {
        self.status == 'U'
    }
}

pub fn status(dir: &Path) -> Vec<Change> {
    let (Some(top), Some(out)) = (
        git(dir, &["rev-parse", "--show-toplevel"], &[0]),
        // Without this, status rewrites .git/index, which the watcher reports, which reruns status.
        git(dir, &["--no-optional-locks", "status", "--porcelain=v1", "-z", "--untracked-files=all"], &[0]),
    ) else {
        return Vec::new();
    };
    parse_status(Path::new(top.trim()), &out)
}

fn parse_status(top: &Path, out: &str) -> Vec<Change> {
    let mut entries = out.split('\0');
    let mut changes = Vec::new();
    while let Some(entry) = entries.next() {
        let Some((code, path)) = entry.split_at_checked(3) else { continue };
        let (x, y) = (code.as_bytes()[0] as char, code.as_bytes()[1] as char);
        if x == 'R' || x == 'C' {
            // The original path follows as its own field.
            entries.next();
        }
        let status = match (x, y) {
            ('?', '?') => 'U',
            // Unmerged paths of a merge or rebase in progress.
            ('U', _) | (_, 'U') | ('A', 'A') | ('D', 'D') => 'C',
            (x, ' ') => x,
            (_, y) => y,
        };
        changes.push(Change {
            path: top.join(path),
            status,
            staged: !matches!(x, ' ' | '?'),
            unstaged: y != ' ',
        });
    }
    changes
}

pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub when: String,
}

pub fn log(dir: &Path, file: Option<&Path>) -> Vec<Commit> {
    let mut args = vec!["log", "-n", "300", "--pretty=format:%h%x1f%s%x1f%an%x1f%ar%x1e"];
    let file = file.map(|f| f.to_string_lossy().into_owned());
    if let Some(file) = &file {
        args.extend(["--follow", "--", file]);
    }
    git(dir, &args, &[0])
        .unwrap_or_default()
        .split('\x1e')
        .filter_map(|record| {
            let mut fields = record.trim_start_matches('\n').split('\x1f');
            Some(Commit {
                hash: fields.next().filter(|h| !h.is_empty())?.to_string(),
                subject: fields.next()?.to_string(),
                author: fields.next()?.to_string(),
                when: fields.next()?.to_string(),
            })
        })
        .collect()
}

pub fn last_message(dir: &Path) -> Option<String> {
    git(dir, &["log", "-1", "--format=%B"], &[0]).map(|m| m.trim_end().to_string())
}

pub fn diff(dir: &Path, file: &Path) -> String {
    let file = file.to_string_lossy();
    git(dir, &["diff", "HEAD", "--", &file], &[0])
        .filter(|d| !d.is_empty())
        // An untracked file has no HEAD side, so diff it against nothing; --no-index exits 1 on a difference.
        .or_else(|| git(dir, &["diff", "--no-index", "--", "/dev/null", &file], &[0, 1]))
        .unwrap_or_default()
}

/// A commit's patch alone, renames detected; the metadata comes from [`commit_info`].
pub fn show(dir: &Path, hash: &str, file: Option<&Path>) -> String {
    let file = file.map(|f| f.to_string_lossy().into_owned());
    let mut args = vec!["show", "--format=", "--patch", "-M", hash];
    if let Some(file) = &file {
        args.extend(["--", file]);
    }
    git(dir, &args, &[0]).unwrap_or_default()
}

pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub when: String,
    pub subject: String,
    pub body: String,
}

pub fn commit_info(dir: &Path, hash: &str) -> Option<CommitInfo> {
    let out = git(dir, &["show", "-s", "--format=%h%x1f%an%x1f%ar · %ad%x1f%s%x1f%b", "--date=format:%Y-%m-%d %H:%M", hash], &[0])?;
    let mut fields = out.trim_end().splitn(5, '\x1f');
    Some(CommitInfo {
        hash: fields.next()?.to_string(),
        author: fields.next()?.to_string(),
        when: fields.next()?.to_string(),
        subject: fields.next()?.to_string(),
        body: fields.next().unwrap_or_default().trim().to_string(),
    })
}

pub fn ahead_behind(dir: &Path) -> Option<(u32, u32)> {
    let out = git(dir, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"], &[0])?;
    let (ahead, behind) = out.trim().split_once('\t')?;
    Some((ahead.parse().ok()?, behind.parse().ok()?))
}

pub fn branches(dir: &Path) -> Vec<String> {
    git(dir, &["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads"], &[0])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LineChange {
    Added,
    Modified,
}

pub fn line_changes(dir: &Path, file: &Path) -> Vec<(Range<u32>, LineChange)> {
    let file = file.to_string_lossy();
    match git(dir, &["diff", "-U0", "--no-color", "HEAD", "--", &file], &[0]) {
        Some(diff) => parse_hunks(&diff),
        None => Vec::new(),
    }
}

fn parse_hunks(diff: &str) -> Vec<(Range<u32>, LineChange)> {
    let count = |s: &str| -> Option<(u32, u32)> {
        let (start, len) = s.split_once(',').unwrap_or((s, "1"));
        Some((start.parse().ok()?, len.parse().ok()?))
    };
    diff.lines()
        .filter_map(|line| {
            let mut parts = line.strip_prefix("@@ -")?.split(' ');
            let (_, old) = count(parts.next()?)?;
            let (start, new) = count(parts.next()?.strip_prefix('+')?)?;
            let kind = if old == 0 { LineChange::Added } else { LineChange::Modified };
            (new > 0).then(|| (start - 1..start - 1 + new, kind))
        })
        .collect()
}

#[std::prelude::rust_2024::test]
fn parses_porcelain_status() {
    let out = " M src/a.rs\0?? new.txt\0R  b.rs\0old.rs\0D  gone.rs\0A  added.rs\0UU both.rs\0";
    let got: Vec<_> = parse_status(Path::new("/r"), out)
        .into_iter()
        .map(|c| (c.path.display().to_string(), c.status))
        .collect();
    let want = [
        ("/r/src/a.rs", 'M'),
        ("/r/new.txt", 'U'),
        ("/r/b.rs", 'R'),
        ("/r/gone.rs", 'D'),
        ("/r/added.rs", 'A'),
        ("/r/both.rs", 'C'),
    ];
    assert_eq!(got, want.map(|(p, s)| (p.to_string(), s)));
}

#[std::prelude::rust_2024::test]
fn parses_hunks() {
    let diff = "diff --git a/x b/x\n@@ -3 +3 @@ fn\n-a\n+b\n@@ -10,0 +11,2 @@\n+c\n+d\n@@ -20,2 +21,0 @@\n-e\n-f\n";
    assert_eq!(
        parse_hunks(diff),
        [(2..3, LineChange::Modified), (10..12, LineChange::Added)]
    );
}

#[std::prelude::rust_2024::test]
fn reads_a_real_repository() {
    let dir = std::env::temp_dir().join(format!("editor-git-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let run = |args: &[&str]| git(&dir, args, &[0]).expect("git");
    run(&["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    run(&["add", "."]);
    run(&["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-qm", "first"]);
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    std::fs::write(dir.join("b.txt"), "new\n").unwrap();

    assert_eq!(branch(&dir).as_deref(), Some("main"));
    let statuses: Vec<_> = status(&dir).iter().map(|c| c.status).collect();
    assert_eq!(statuses, ['M', 'U']);
    assert_eq!(log(&dir, None)[0].subject, "first");
    assert_eq!(line_changes(&dir, Path::new("a.txt")), [(1..3, LineChange::Modified)]);
    assert!(diff(&dir, Path::new("b.txt")).contains("+new"));
    std::fs::remove_dir_all(&dir).unwrap();
}
