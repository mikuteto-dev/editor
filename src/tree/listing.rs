use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use super::Row;
use crate::git::Change;

/// One folder level, folders first, without what .gitignore hides.
pub(super) fn read_dir(dir: &Path, depth: usize) -> Vec<Row> {
    let mut rows: Vec<Row> = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git" && e.file_name() != ".DS_Store")
        .build()
        .skip(1)
        .filter_map(Result::ok)
        .map(|e| {
            let dir = e
                .file_type()
                .is_some_and(|t| t.is_dir() || t.is_symlink() && e.path().is_dir());
            Row {
                name: e.file_name().to_string_lossy().into_owned().into(),
                path: e.into_path(),
                depth,
                dir,
                open: false,
            }
        })
        .collect();
    rows.sort_by_cached_key(|r| (!r.dir, r.name.to_lowercase()));
    rows
}

pub(super) fn merge(old: Vec<Row>, fresh: Vec<Row>, depth: usize) -> Vec<Row> {
    let mut kept: HashMap<PathBuf, Vec<Row>> = HashMap::new();
    let mut current = None;
    for row in old {
        if row.depth == depth {
            current = Some(row.path.clone());
        }
        if let Some(path) = &current {
            kept.entry(path.clone()).or_default().push(row);
        }
    }
    fresh
        .into_iter()
        .flat_map(|row| kept.remove(&row.path).unwrap_or_else(|| vec![row]))
        .collect()
}

pub(super) fn status_map(root: &Path, changes: &[Change]) -> HashMap<PathBuf, char> {
    let mut map = HashMap::new();
    for change in changes {
        map.insert(change.path.clone(), change.status);
        for dir in change.path.ancestors().skip(1).take_while(|d| d.starts_with(root) && *d != root) {
            // A folder with mixed changes reads as modified.
            let status = map.entry(dir.to_path_buf()).or_insert(change.status);
            if *status != change.status {
                *status = 'M';
            }
        }
    }
    map
}

#[std::prelude::rust_2024::test]
fn reads_one_level_sorted_and_gitignored() {
    let dir = std::env::temp_dir().join(format!("editor-tree-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("src/inner")).unwrap();
    for f in ["b.rs", "A.md", "skip.log", ".gitignore"] {
        std::fs::write(dir.join(f), "*.log\n").unwrap();
    }
    let names: Vec<_> = read_dir(&dir, 0).iter().map(|r| r.name.to_string()).collect();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(names, ["src", ".gitignore", "A.md", "b.rs"]);
}

#[std::prelude::rust_2024::test]
fn merge_keeps_open_subtrees() {
    let row = |path: &str, depth, open| Row {
        path: path.into(),
        name: path.into(),
        depth,
        dir: true,
        open,
    };
    let old = vec![row("a", 0, true), row("a/x", 1, false), row("b", 0, false), row("gone", 0, false)];
    let fresh = vec![row("a", 0, false), row("b", 0, false), row("new", 0, false)];
    let names: Vec<_> = merge(old, fresh, 0).iter().map(|r| (r.name.to_string(), r.open)).collect();
    let expect = [("a", true), ("a/x", false), ("b", false), ("new", false)];
    assert_eq!(names, expect.map(|(n, o)| (n.to_string(), o)));
}

#[std::prelude::rust_2024::test]
fn folders_take_the_status_inside() {
    let change = |path: &str, status| Change { path: path.into(), status, staged: false, unstaged: true };
    let map = status_map(Path::new("/r"), &[change("/r/a/new.rs", 'U'), change("/r/b/c/x.rs", 'U'), change("/r/b/y.rs", 'M')]);
    assert_eq!(map[Path::new("/r/a")], 'U');
    assert_eq!(map[Path::new("/r/b/c")], 'U');
    assert_eq!(map[Path::new("/r/b")], 'M');
    assert!(!map.contains_key(Path::new("/r")));
}
