use std::path::{Path, PathBuf};

use super::{Change, git};
use crate::{files::trash, shell};

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
        'U' => trash(&change.path).map(|_| ()),
        'A' => {
            write(dir, &args_with_paths(&["rm", "--cached", "-q"], &path), None)?;
            trash(&change.path).map(|_| ())
        }
        _ => write(dir, &args_with_paths(&["restore", "--source=HEAD", "--staged", "--worktree"], &path), None),
    }
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

pub fn switch(dir: &Path, branch: &str) -> Result<(), String> {
    write(dir, &["switch", branch], None)
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

#[std::prelude::rust_2024::test]
fn stages_commits_and_discards() {
    use super::{blame, branch, commit_info, last_message, log, show, status};
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
