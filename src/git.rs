//! Git through the `git` CLI: one process per command, nothing resident.

use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use crate::shell;

fn git(dir: &Path, args: &[&str], ok: &[i32]) -> Option<String> {
    let out = shell::command("git").arg("-C").arg(dir).args(args).output().ok()?;
    ok.contains(&out.status.code()?)
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn branch(dir: &Path) -> Option<String> {
    // symbolic-ref also names the branch of a repository without commits.
    git(dir, &["symbolic-ref", "--short", "HEAD"], &[0])
        .or_else(|| git(dir, &["rev-parse", "--short", "HEAD"], &[0]))
        .map(|s| s.trim().to_string())
}

pub struct Change {
    pub path: PathBuf,
    pub status: char,
    /// Some of the change is in the index.
    pub staged: bool,
    /// Some of the change is only in the working tree.
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

/// Runs a command that changes the repository; the error is git's own message.
fn write(dir: &Path, args: &[&str], stdin: Option<&str>) -> Result<(), String> {
    use std::io::Write;
    let mut child = shell::command("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // A credential prompt would hang with no terminal to answer it.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(text) = stdin {
        child.stdin.take().unwrap().write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn args_with_paths<'a>(args: &[&'a str], paths: &'a [String]) -> Vec<&'a str> {
    args.iter().copied().chain(["--"]).chain(paths.iter().map(String::as_str)).collect()
}

fn strings(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| p.to_string_lossy().into_owned()).collect()
}

pub fn stage(dir: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let paths = strings(paths);
    // -A also records deletions.
    write(dir, &args_with_paths(&["add", "-A"], &paths), None)
}

pub fn unstage(dir: &Path, paths: &[PathBuf]) -> Result<(), String> {
    let paths = strings(paths);
    if git(dir, &["rev-parse", "--verify", "-q", "HEAD"], &[0]).is_some() {
        write(dir, &args_with_paths(&["reset", "-q"], &paths), None)
    } else {
        // Before the first commit there is no HEAD to reset to.
        write(dir, &args_with_paths(&["rm", "--cached", "-r", "-q"], &paths), None)
    }
}

/// Puts the file back to HEAD. A file HEAD does not have goes to the Trash, so it can be recovered.
pub fn discard(dir: &Path, change: &Change) -> Result<(), String> {
    let path = strings(std::slice::from_ref(&change.path));
    match change.status {
        'U' => trash(&change.path),
        'A' => {
            write(dir, &args_with_paths(&["rm", "--cached", "-q"], &path), None)?;
            trash(&change.path)
        }
        _ => write(dir, &args_with_paths(&["restore", "--source=HEAD", "--staged", "--worktree"], &path), None),
    }
}

fn trash(path: &Path) -> Result<(), String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, None)
        .map_err(|e| e.localizedDescription().to_string())
}

/// With nothing staged, commits every tracked change, as Zed does.
pub fn commit(dir: &Path, message: &str, tracked: bool, amend: bool) -> Result<(), String> {
    let mut args = vec!["commit", "-F", "-"];
    if tracked {
        args.push("-a");
    }
    if amend {
        args.push("--amend");
    }
    write(dir, &args, Some(message))
}

pub fn last_message(dir: &Path) -> Option<String> {
    git(dir, &["log", "-1", "--format=%B"], &[0]).map(|m| m.trim_end().to_string())
}

pub fn create_branch(dir: &Path, branch: &str) -> Result<(), String> {
    write(dir, &["switch", "-c", branch], None)
}

pub fn stash(dir: &Path) -> Result<(), String> {
    write(dir, &["stash", "push", "--include-untracked"], None)
}

pub fn stash_pop(dir: &Path) -> Result<(), String> {
    write(dir, &["stash", "pop"], None)
}

/// Who last changed each line, as "author, when · summary"; lines not committed yet are None.
pub fn blame(dir: &Path, file: &Path) -> Vec<Option<String>> {
    let file = file.to_string_lossy();
    let Some(out) = git(dir, &["blame", "--porcelain", "--", &file], &[0]) else { return Vec::new() };
    parse_blame(&out, now())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn parse_blame(out: &str, now: u64) -> Vec<Option<String>> {
    // Porcelain output gives a commit's details only the first time it appears.
    let mut commits: std::collections::HashMap<&str, (String, u64, String)> = std::collections::HashMap::new();
    let mut lines = Vec::new();
    let mut current = "";
    for line in out.lines() {
        if line.starts_with('\t') {
            let info = commits.get(current).filter(|_| !current.starts_with("0000000"));
            lines.push(info.map(|(author, time, summary)| format!("{author}, {} · {summary}", ago(now.saturating_sub(*time)))));
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key.len() == 40 && key.chars().all(|c| c.is_ascii_hexdigit()) {
            current = key;
            commits.entry(key).or_default();
        } else if let Some(entry) = commits.get_mut(current) {
            match key {
                "author" => entry.0 = value.to_string(),
                "author-time" => entry.1 = value.parse().unwrap_or_default(),
                "summary" => entry.2 = value.to_string(),
                _ => {}
            }
        }
    }
    lines
}

fn ago(secs: u64) -> String {
    let (n, unit) = match secs {
        s if s < 3600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s if s < 30 * 86_400 => (s / 86_400, "day"),
        s if s < 365 * 86_400 => (s / (30 * 86_400), "month"),
        s => (s / (365 * 86_400), "year"),
    };
    match n {
        0 => "just now".to_string(),
        1 => format!("1 {unit} ago"),
        n => format!("{n} {unit}s ago"),
    }
}

pub fn fetch(dir: &Path) -> Result<(), String> {
    write(dir, &["fetch", "--prune"], None)
}

pub fn pull(dir: &Path) -> Result<(), String> {
    write(dir, &["pull"], None)
}

pub fn push(dir: &Path) -> Result<(), String> {
    if git(dir, &["rev-parse", "--abbrev-ref", "@{upstream}"], &[0]).is_some() {
        return write(dir, &["push"], None);
    }
    // A new branch has no upstream yet; publish it to the first remote.
    let remotes = git(dir, &["remote"], &[0]).unwrap_or_default();
    let remote = remotes.lines().find(|r| *r == "origin").or(remotes.lines().next());
    let remote = remote.ok_or("No remote to push to")?;
    write(dir, &["push", "-u", remote, "HEAD"], None)
}

/// Commits to push and to pull, when the branch tracks a remote one.
pub fn ahead_behind(dir: &Path) -> Option<(u32, u32)> {
    let out = git(dir, &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"], &[0])?;
    let (ahead, behind) = out.trim().split_once('\t')?;
    Some((ahead.parse().ok()?, behind.parse().ok()?))
}

/// Local branches, most recently committed first.
pub fn branches(dir: &Path) -> Vec<String> {
    git(dir, &["for-each-ref", "--sort=-committerdate", "--format=%(refname:short)", "refs/heads"], &[0])
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

pub fn switch(dir: &Path, branch: &str) -> Result<(), String> {
    write(dir, &["switch", branch], None)
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

#[std::prelude::rust_2024::test]
fn stages_commits_and_discards() {
    let dir = std::env::temp_dir().join(format!("editor-git-write-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Status reports real paths; the temp dir sits behind a symlink on macOS.
    let dir = dir.canonicalize().unwrap();
    let run = |args: &[&str]| git(&dir, args, &[0]).expect("git");
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.name", "t"]);
    run(&["config", "user.email", "t@example.com"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    std::fs::write(dir.join("b.txt"), "new\n").unwrap();
    let find = |name: &str| status(&dir).into_iter().find(|c| c.path.ends_with(name));

    // Before the first commit, unstaging has no HEAD to fall back on.
    stage(&dir, &[dir.join("a.txt"), dir.join("b.txt")]).unwrap();
    unstage(&dir, &[dir.join("b.txt")]).unwrap();
    assert!(find("b.txt").unwrap().untracked());
    let a = find("a.txt").unwrap();
    assert!(a.staged && !a.unstaged);

    commit(&dir, "first\n\nbody", false, false).unwrap();
    assert_eq!(log(&dir, None)[0].subject, "first");
    let info = commit_info(&dir, "HEAD").unwrap();
    assert_eq!((info.author.as_str(), info.subject.as_str(), info.body.as_str()), ("t", "first", "body"));
    let patch = show(&dir, "HEAD", None);
    assert!(patch.starts_with("diff --git a/a.txt b/a.txt"), "{patch}");
    assert!(find("a.txt").is_none());
    assert_eq!(blame(&dir, Path::new("a.txt"))[0].as_deref().map(|b| b.starts_with("t, ")), Some(true));

    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    assert_eq!(blame(&dir, Path::new("a.txt")), [None]);
    commit(&dir, "tracked", true, false).unwrap();
    assert_eq!(log(&dir, None)[0].subject, "tracked");
    assert!(find("b.txt").unwrap().untracked());
    commit(&dir, "renamed", false, true).unwrap();
    assert_eq!(last_message(&dir).as_deref(), Some("renamed"));
    assert_eq!(log(&dir, None).len(), 2);

    stash(&dir).unwrap();
    assert!(status(&dir).is_empty());
    stash_pop(&dir).unwrap();
    assert!(find("b.txt").unwrap().untracked());
    create_branch(&dir, "feature").unwrap();
    assert_eq!(branch(&dir).as_deref(), Some("feature"));

    std::fs::write(dir.join("a.txt"), "oops\n").unwrap();
    stage(&dir, &[dir.join("a.txt")]).unwrap();
    discard(&dir, &find("a.txt").unwrap()).unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "changed\n");
    assert!(find("a.txt").is_none());
    assert!(commit(&dir, "nothing", false, false).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
