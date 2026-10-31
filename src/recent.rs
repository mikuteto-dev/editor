use std::path::{Path, PathBuf};

use crate::config_dir;

const LIMIT: usize = 50;

fn file() -> PathBuf {
    config_dir().join("recent")
}

pub fn load() -> Vec<PathBuf> {
    std::fs::read_to_string(file())
        .unwrap_or_default()
        .lines()
        .map(PathBuf::from)
        .collect()
}

pub fn push(dir: &Path) -> std::io::Result<()> {
    let list = touch(load(), dir);
    std::fs::create_dir_all(config_dir())?;
    let text: Vec<_> = list.iter().map(|p| p.to_string_lossy()).collect();
    std::fs::write(file(), text.join("\n"))
}

fn touch(mut list: Vec<PathBuf>, dir: &Path) -> Vec<PathBuf> {
    list.retain(|p| p != dir);
    list.insert(0, dir.to_path_buf());
    list.truncate(LIMIT);
    list
}

#[std::prelude::rust_2024::test]
fn touch_moves_to_front_and_caps() {
    let list: Vec<PathBuf> = (0..LIMIT).map(|i| PathBuf::from(format!("/{i}"))).collect();
    let list = touch(list, Path::new("/3"));
    assert_eq!(list[0], Path::new("/3"));
    assert_eq!(list.len(), LIMIT);
    let list = touch(list, Path::new("/new"));
    assert_eq!(list[0], Path::new("/new"));
    assert_eq!(list.len(), LIMIT);
    assert_eq!(list.iter().filter(|p| *p == Path::new("/3")).count(), 1);
}
