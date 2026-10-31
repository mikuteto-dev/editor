mod drag;
mod edit;
mod listing;
mod render;
#[cfg(test)]
mod tests;
mod undo;

use std::{collections::HashMap, path::PathBuf};

use gpui_kit::{
    component::input::{Redo, Undo},
    *,
};

use crate::git::Change;
use edit::Editing;
use listing::{merge, read_dir, status_map};

actions!(tree, [Up, Down, Expand, Collapse, Enter, CancelEdit]);

pub fn init(cx: &mut App) {
    let context = Some("FileTree");
    cx.bind_keys([
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("right", Expand, context),
        KeyBinding::new("left", Collapse, context),
        KeyBinding::new("enter", Enter, context),
        KeyBinding::new("escape", CancelEdit, context),
        // The editor's own actions, so Edit ▸ Undo in the menu bar reaches the tree too.
        KeyBinding::new("cmd-z", Undo, context),
        KeyBinding::new("cmd-shift-z", Redo, context),
    ]);
}

pub struct FileTree {
    root: PathBuf,
    rows: Vec<Row>,
    selected: Option<PathBuf>,
    status: HashMap<PathBuf, char>,
    editing: Option<Editing>,
    spring: drag::Spring,
    history: undo::History,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

struct Row {
    path: PathBuf,
    name: SharedString,
    depth: usize,
    dir: bool,
    open: bool,
}

pub enum TreeEvent {
    Open { path: PathBuf, pin: bool },
    Renamed { from: PathBuf, to: PathBuf },
    Deleted(PathBuf),
}

impl EventEmitter<TreeEvent> for FileTree {}

impl FileTree {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut tree = Self {
            root: root.clone(),
            rows: Vec::new(),
            selected: None,
            status: HashMap::new(),
            editing: None,
            spring: Default::default(),
            history: Default::default(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        };
        tree.load(root, None, 0, cx);
        tree
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    fn selected_ix(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows.iter().position(|r| &r.path == selected)
    }

    fn move_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(ix) {
            self.selected = Some(row.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let ix = match self.selected_ix() {
            Some(ix) => ix.saturating_add_signed(delta).min(self.rows.len().saturating_sub(1)),
            None => 0,
        };
        self.move_to(ix, cx);
    }

    fn fold(&mut self, open: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_ix() else { return };
        let row = &self.rows[ix];
        if row.dir && row.open != open {
            return self.click(ix, false, cx);
        }
        if !open && let Some(parent) = self.rows[..ix].iter().rposition(|r| r.depth < row.depth) {
            self.move_to(parent, cx);
        }
    }

    pub fn set_status(&mut self, changes: &[Change], cx: &mut Context<Self>) {
        self.status = status_map(&self.root, changes);
        cx.notify();
    }

    pub fn select(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.selected = path;
        cx.notify();
    }

    fn load(&mut self, dir: PathBuf, parent: Option<PathBuf>, depth: usize, cx: &mut Context<Self>) {
        let entries = cx.background_spawn(async move { read_dir(&dir, depth) });
        cx.spawn(async move |this, cx| {
            let entries = entries.await;
            this.update(cx, |this, cx| {
                let at = match &parent {
                    None if this.rows.is_empty() => 0,
                    None => return,
                    Some(parent) => {
                        let Some(ix) = this.rows.iter().position(|r| &r.path == parent) else {
                            return;
                        };
                        // Collapsed while loading, or loaded twice by a quick re-open.
                        let loaded = this.rows.get(ix + 1).is_some_and(|r| r.depth > depth - 1);
                        if !this.rows[ix].open || loaded {
                            return;
                        }
                        ix + 1
                    }
                };
                this.rows.splice(at..at, entries);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn click(&mut self, ix: usize, pin: bool, cx: &mut Context<Self>) {
        let row = &mut self.rows[ix];
        if !row.dir {
            let path = row.path.clone();
            self.selected = Some(path.clone());
            cx.emit(TreeEvent::Open { path, pin });
            cx.notify();
            return;
        }
        row.open = !row.open;
        if row.open {
            let (path, depth) = (row.path.clone(), row.depth + 1);
            self.load(path.clone(), Some(path), depth, cx);
        } else {
            let end = self.subtree_end(ix);
            self.rows.drain(ix + 1..end);
        }
        cx.notify();
    }

    fn subtree_end(&self, ix: usize) -> usize {
        let depth = self.rows[ix].depth;
        self.rows[ix + 1..]
            .iter()
            .position(|r| r.depth <= depth)
            .map_or(self.rows.len(), |n| ix + 1 + n)
    }

    // shortcut: reads on the main thread; fine for directory listings, move to background_spawn if huge dirs stutter.
    pub fn refresh(&mut self, dirs: &[PathBuf], cx: &mut Context<Self>) {
        for dir in dirs {
            let span = if *dir == self.root {
                Some((0, self.rows.len(), 0))
            } else {
                self.rows
                    .iter()
                    .position(|r| r.path == *dir && r.open)
                    .map(|ix| (ix + 1, self.subtree_end(ix), self.rows[ix].depth + 1))
            };
            let Some((start, end, depth)) = span else { continue };
            let old: Vec<Row> = self.rows.drain(start..end).collect();
            let rows = merge(old, read_dir(dir, depth), depth);
            self.rows.splice(start..start, rows);
        }
        cx.notify();
    }
}
