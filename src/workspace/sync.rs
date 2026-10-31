use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use gpui_kit::*;

use super::{Panel, Workspace};
use crate::tab::Body;

fn in_git(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == ".git")
}

impl Workspace {
    pub(super) fn changed(&mut self, paths: HashSet<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if paths.iter().any(|p| in_git(p) && p.ends_with("HEAD")) {
            self.load_branch(cx);
        }
        // Commits, checkouts and staging move HEAD or the index, which can change every open file's marks.
        let repo_moved = paths.iter().any(|p| in_git(p) && (p.ends_with("index") || p.ends_with("HEAD")));
        for ix in 0..self.tabs.len() {
            if repo_moved || paths.contains(&self.tabs[ix].path) {
                self.mark_changes(ix, cx);
            }
        }
        self.load_status(cx);
        if let Some((git, _)) = &self.git
            && self.sidebar
            && self.panel == Panel::Git
        {
            git.update(cx, |git, cx| git.refresh(cx));
        }

        let mut dirs: Vec<PathBuf> = paths
            .iter()
            .flat_map(|p| std::iter::once(p.as_path()).chain(p.parent()))
            .filter(|p| !in_git(p))
            .map(Path::to_path_buf)
            .collect();
        dirs.sort();
        dirs.dedup();
        if let Some((tree, _)) = &self.tree {
            tree.update(cx, |tree, cx| tree.refresh(&dirs, cx));
        }

        for tab in &self.tabs {
            let Body::Text(text) = &tab.body else { continue };
            if text.dirty || !paths.contains(&tab.path) {
                continue;
            }
            let Ok(disk) = std::fs::read_to_string(&tab.path) else { continue };
            text.editor.update(cx, |editor, cx| {
                // Our own saves come back here too; leave identical text untouched.
                if editor.text().to_string() != disk {
                    let pos = editor.cursor_position();
                    editor.set_value(disk, window, cx);
                    editor.set_cursor_position(pos, window, cx);
                }
            });
        }
    }
}
