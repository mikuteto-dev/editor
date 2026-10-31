use std::path::{Path, PathBuf};

use gpui_kit::{
    component::{
        ActiveTheme,
        command::CommandItem,
        input::{Position, RangeDecoration, RangeDecorationStyle, RopeExt},
    },
    *,
};

use super::Workspace;
use crate::{
    commands::FileHistory,
    git::{self, LineChange},
    palette,
    tab::{Body, Tab},
};

impl Workspace {
    pub(super) fn load_branch(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        cx.spawn(async move |this, cx| {
            let branch = cx.background_spawn(async move { git::branch(&root) }).await;
            this.update(cx, |this, cx| {
                this.branch = branch.map(Into::into);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// One status run feeds the tree colors, the Git panel and the badge.
    pub(super) fn load_status(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        // shortcut: reruns on every watcher batch, gitignored build output included; filter those paths if builds show up in CPU.
        self.status = cx.spawn(async move |this, cx| {
            let changes = cx.background_spawn(async move { git::status(&root) }).await;
            this.update(cx, |this, cx| {
                this.change_count = changes.len();
                if let Some((tree, _)) = &this.tree {
                    tree.update(cx, |tree, cx| tree.set_status(&changes, cx));
                }
                if let Some((git, _)) = &this.git {
                    git.update(cx, |git, cx| git.set_changes(changes, cx));
                }
                cx.notify();
            })
            .ok();
        });
    }

    /// Tints lines that differ from HEAD. The editor moves the ranges along
    /// with edits, so this reruns only on save and repository changes.
    pub(super) fn mark_changes(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let Some(Body::Text(text)) = self.tabs.get(ix).map(|t| &t.body) else { return };
        if text.readonly {
            return;
        }
        let (path, editor) = (self.tabs[ix].path.clone(), text.editor.clone());
        cx.spawn(async move |this, cx| {
            let changes = cx.background_spawn(async move { git::line_changes(&root, &path) }).await;
            this.update(cx, |this, cx| {
                let (green, blue) = (cx.theme().green.opacity(0.22), cx.theme().blue.opacity(0.22));
                let rope = editor.read(cx).text().clone();
                let decorations = changes
                    .into_iter()
                    .map(|(lines, kind)| {
                        let start = rope.position_to_offset(&Position::new(lines.start, 0));
                        let end = rope.position_to_offset(&Position::new(lines.end, 0));
                        let color = match kind {
                            LineChange::Added => green,
                            LineChange::Modified => blue,
                        };
                        RangeDecoration::new(start..end)
                            .with_style(RangeDecorationStyle::Fill)
                            .with_color(color)
                    })
                    .collect();
                let Some(text) = this.text_mut(&editor) else { return };
                match &text.git_marks {
                    Some(marks) => marks.set(decorations, cx),
                    None => {
                        text.git_marks = Some(editor.update(cx, |editor, cx| {
                            editor.create_range_decorations_collection(decorations, cx)
                        }))
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_diff(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let title = format!("{name} (diff)");
        let diff = git::diff(&root, &path);
        // A synthetic path keys the tab so it never collides with the file itself.
        let key = PathBuf::from(format!("git-diff:{}", path.display()));
        // Always rebuilt: the working copy may have changed since the last look.
        self.tabs.retain(|t| t.path != key);
        let body = self.text(&key, diff, "diff", true, window, cx);
        self.insert(Tab { path: key, name: title.into(), body, pinned: false }, window, cx);
    }

    pub(super) fn open_commit(
        &mut self,
        hash: String,
        file: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.root.clone() else { return };
        let file_key = file.as_deref().unwrap_or(Path::new("")).display();
        let key = PathBuf::from(format!("git-show:{hash}:{file_key}"));
        if let Some(ix) = self.tabs.iter().position(|t| t.path == key) {
            return self.activate(ix, window, cx);
        }
        let text = git::show(&root, &hash, file.as_deref());
        let body = self.text(&key, text, "diff", true, window, cx);
        self.insert(Tab { path: key, name: hash.into(), body, pinned: false }, window, cx);
    }

    pub(super) fn file_history(&mut self, _: &FileHistory, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(root), Some(tab)) = (self.root.clone(), self.tabs.get(self.active)) else { return };
        let file = tab.path.clone();
        let commits = git::log(&root, Some(&file));
        if commits.is_empty() {
            return;
        }
        let items = commits
            .iter()
            .map(|c| CommandItem::new().label(format!("{}  {}  {} · {}", c.hash, c.subject, c.author, c.when)))
            .collect();
        let this = cx.entity().downgrade();
        palette::open(
            items,
            Some(0),
            |_, _, _| {},
            move |ix, window, cx| {
                let hash = commits[ix].hash.clone();
                let file = file.clone();
                this.update(cx, |this, cx| this.open_commit(hash, Some(file), window, cx)).ok();
            },
            |_, _| {},
            window,
            cx,
        );
    }
}
