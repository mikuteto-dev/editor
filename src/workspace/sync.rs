use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use gpui_kit::{component::input::Rope, *};

use super::{Panel, Workspace};
use crate::{licenses, tab::Body};

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

        for tab in &mut self.tabs {
            let Body::Text(text) = &mut tab.body else { continue };
            if text.dirty || !paths.contains(&tab.path) {
                continue;
            }
            let Ok(disk) = std::fs::read_to_string(&tab.path) else { continue };
            let rope = Rope::from_str(&disk);
            // Our own saves come back here too; leave identical text untouched.
            if *text.editor.read(cx).text() == rope {
                continue;
            }
            text.saved = rope;
            text.editor.update(cx, |editor, cx| {
                let pos = editor.cursor_position();
                editor.set_value(disk, window, cx);
                editor.set_cursor_position(pos, window, cx);
            });
        }

        // A manifest or lockfile change can change any package's dependencies.
        let deps_moved = paths.iter().any(|p| !in_git(p) && licenses::affects(p));
        if deps_moved {
            self.reports.clear();
        }
        for ix in 0..self.tabs.len() {
            if deps_moved || paths.contains(&self.tabs[ix].path) {
                self.check_licenses(ix, cx);
            }
        }
    }
}
