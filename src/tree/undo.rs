//! Undoing and redoing what was done to files from the tree. Every step is a set of moves:
//! a rename or drag moves an entry, a delete moves it into the Trash, and undoing a new file
//! moves it there too, so each can be moved back.

use std::path::{Path, PathBuf};

use gpui_kit::{component::WindowExt, *};

use super::{FileTree, TreeEvent};
use crate::files;

pub(super) enum Op {
    Move { from: PathBuf, to: PathBuf },
    /// A new entry; undoing it sends it to the Trash.
    Create(PathBuf),
}

#[derive(Default)]
pub(super) struct History {
    done: Vec<Vec<Op>>,
    undone: Vec<Vec<Op>>,
}

impl History {
    pub(super) fn record(&mut self, step: Vec<Op>) {
        if !step.is_empty() {
            self.done.push(step);
            self.undone.clear();
        }
    }
}

fn reverse(op: Op) -> Result<Op, String> {
    match op {
        Op::Move { from, to } => {
            files::move_back(&to, &from)?;
            Ok(Op::Move { from: to, to: from })
        }
        Op::Create(path) => {
            let trashed = files::trash(&path)?;
            Ok(Op::Move { from: trashed, to: path })
        }
    }
}

impl FileTree {
    pub(super) fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(step) = self.history.done.pop() {
            let step = self.run_backwards(step, window, cx);
            self.history.undone.push(step);
        }
    }

    pub(super) fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(step) = self.history.undone.pop() {
            let step = self.run_backwards(step, window, cx);
            self.history.done.push(step);
        }
    }

    fn run_backwards(&mut self, step: Vec<Op>, window: &mut Window, cx: &mut Context<Self>) -> Vec<Op> {
        let mut reversed = Vec::new();
        let mut dirs = Vec::new();
        for op in step.into_iter().rev() {
            match reverse(op) {
                Ok(Op::Move { from, to }) => {
                    dirs.extend(from.parent().map(Path::to_path_buf));
                    dirs.extend(to.parent().map(Path::to_path_buf));
                    // Into the Trash reads as a delete; anything else, tabs follow the move.
                    let event = if to.starts_with(trash_dir()) {
                        TreeEvent::Deleted(from.clone())
                    } else {
                        TreeEvent::Renamed { from: from.clone(), to: to.clone() }
                    };
                    cx.emit(event);
                    reversed.push(Op::Move { from, to });
                }
                Ok(op) => reversed.push(op),
                Err(e) => window.push_notification(e, cx),
            }
        }
        dirs.sort();
        dirs.dedup();
        self.refresh(&dirs, cx);
        reversed.reverse();
        reversed
    }
}

fn trash_dir() -> PathBuf {
    std::env::home_dir().unwrap_or_default().join(".Trash")
}

#[std::prelude::rust_2024::test]
fn moves_reverse_and_reverse_again() {
    let dir = std::env::temp_dir().join(format!("editor-undo-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    let to = files::move_into(&dir.join("a.txt"), &dir.join("sub")).unwrap();
    let undone = reverse(Op::Move { from: dir.join("a.txt"), to }).unwrap();
    assert!(dir.join("a.txt").is_file() && !dir.join("sub/a.txt").exists());
    reverse(undone).unwrap();
    assert!(dir.join("sub/a.txt").is_file() && !dir.join("a.txt").exists());
    // Something new in the old place is never overwritten.
    std::fs::write(dir.join("a.txt"), "new").unwrap();
    assert!(reverse(Op::Move { from: dir.join("a.txt"), to: dir.join("sub/a.txt") }).is_err());
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "new");
    std::fs::remove_dir_all(&dir).unwrap();
}
