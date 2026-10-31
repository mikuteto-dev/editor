use std::path::{Path, PathBuf};

use crate::config_dir;

const LIMIT: usize = 50;

#[derive(Debug, PartialEq)]
pub struct Session {
    pub tabs: Vec<PathBuf>,
    pub active: usize,
}

fn file() -> PathBuf {
    config_dir().join("sessions")
}

pub fn load(root: &Path) -> Option<Session> {
    let text = std::fs::read_to_string(file()).ok()?;
    text.lines().filter_map(parse).find(|(r, _)| r == root).map(|(_, s)| s)
}

pub fn save(root: &Path, session: &Session) -> std::io::Result<()> {
    let text = put(&std::fs::read_to_string(file()).unwrap_or_default(), root, session);
    std::fs::create_dir_all(config_dir())?;
    std::fs::write(file(), text)
}

// shortcut: a path containing a tab or newline is stored wrong; move to a structured format if that ever matters.
fn parse(line: &str) -> Option<(PathBuf, Session)> {
    let mut fields = line.split('\t');
    let root = PathBuf::from(fields.next()?);
    let active = fields.next()?.parse().ok()?;
    Some((root, Session { tabs: fields.map(PathBuf::from).collect(), active }))
}

fn put(text: &str, root: &Path, session: &Session) -> String {
    let mut line = format!("{}\t{}", root.to_string_lossy(), session.active);
    for tab in &session.tabs {
        line.push('\t');
        line.push_str(&tab.to_string_lossy());
    }
    let others = text.lines().filter(|l| parse(l).is_some_and(|(r, _)| r != root));
    let lines: Vec<_> = std::iter::once(line.as_str()).chain(others).take(LIMIT).collect();
    lines.join("\n")
}

#[std::prelude::rust_2024::test]
fn put_replaces_the_folder_and_keeps_others() {
    let a = Session { tabs: vec!["/a/x.rs".into(), "/a/y z.md".into()], active: 1 };
    let b = Session { tabs: vec![], active: 0 };
    let text = put("", Path::new("/a"), &a);
    let text = put(&text, Path::new("/b"), &b);
    let a2 = Session { tabs: vec!["/a/w.rs".into()], active: 0 };
    let text = put(&text, Path::new("/a"), &a2);
    let all: Vec<_> = text.lines().filter_map(parse).collect();
    assert_eq!(all, vec![(PathBuf::from("/a"), a2), (PathBuf::from("/b"), b)]);
}
