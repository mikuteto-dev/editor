use std::path::{Path, PathBuf};

use gpui_kit::{
    component::{
        ActiveTheme, WindowExt,
        command::CommandItem,
        input::{EditorState, Input, InputEvent, InputState},
        input::{Position, RangeDecoration, RangeDecorationStyle, RopeExt},
    },
    *,
};

use super::Workspace;
use crate::{
    commands::FileHistory,
    diff_view::{DiffView, Part},
    git::{self, CommitInfo, LineChange},
    git_panel::GitPanel,
    palette,
    tab::{Body, Tab},
};

impl Workspace {
    pub(super) fn load_branch(&mut self, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        cx.spawn(async move |this, cx| {
            let (branch, ahead_behind) =
                cx.background_spawn(async move { (git::branch(&root), git::ahead_behind(&root)) }).await;
            this.update(cx, |this, cx| {
                this.branch = branch.map(Into::into);
                this.ahead_behind = ahead_behind;
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
            let (changes, blame) = cx
                .background_spawn(async move { (git::line_changes(&root, &path), git::blame(&root, &path)) })
                .await;
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
                if let Some(text) = this.text_mut(&editor) {
                    text.blame = blame.into_iter().map(|b| b.map(Into::into)).collect();
                }
                this.set_marks(&editor, decorations, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_marks(&mut self, editor: &Entity<EditorState>, decorations: Vec<RangeDecoration>, cx: &mut Context<Self>) {
        let Some(text) = self.text_mut(editor) else { return };
        match &text.git_marks {
            Some(marks) => marks.set(decorations, cx),
            None => {
                text.git_marks =
                    Some(editor.update(cx, |editor, cx| editor.create_range_decorations_collection(decorations, cx)))
            }
        }
    }


    pub(super) fn open_diff(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let title = format!("{name} (diff)");
        // A synthetic path keys the tab so it never collides with the file itself.
        let key = PathBuf::from(format!("git-diff:{}", path.display()));
        let diff = cx.background_spawn(async move { (None, DiffView::load(&git::diff(&root, &path))) });
        // Always rebuilt: the working copy may have changed since the last look.
        self.open_patch(key, title, diff, window, cx);
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
        let title = hash.clone();
        let loaded = cx.background_spawn(async move {
            (git::commit_info(&root, &hash), DiffView::load(&git::show(&root, &hash, file.as_deref())))
        });
        self.open_patch(key, title, loaded, window, cx);
    }

    /// Opens a diff tab once git has produced and parsed it, without holding up the window meanwhile.
    fn open_patch(
        &mut self,
        key: PathBuf,
        tab_name: String,
        loaded: Task<(Option<CommitInfo>, Vec<Part>)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            let (info, files) = loaded.await;
            this.update_in(cx, |this, window, cx| {
                this.tabs.retain(|t| t.path != key);
                let title = info.as_ref().map_or_else(|| tab_name.clone(), |i| i.subject.clone());
                let view = cx.new(|cx| DiffView::new(title.into(), info, files, cx));
                this.insert(Tab { path: key, name: tab_name.into(), body: Body::Diff(view), pinned: false }, window, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Runs a remote operation through the panel, so its progress shows there.
    pub(super) fn with_git(
        &mut self,
        op: fn(&mut GitPanel, &mut Window, &mut Context<GitPanel>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((git, _)) = &self.git {
            git.update(cx, |git, cx| op(git, window, cx));
        }
    }

    pub(super) fn new_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("New branch name"));
        cx.subscribe_in(&input, window, move |this, input, e: &InputEvent, window, cx| {
            let InputEvent::PressEnter { .. } = e else { return };
            let name = input.read(cx).value().trim().to_string();
            if name.is_empty() {
                return;
            }
            if let Err(e) = git::create_branch(&root, &name) {
                return window.push_notification(e, cx);
            }
            window.close_dialog(cx);
            this.load_branch(cx);
            this.with_git(|git, _, cx| git.refresh(cx), window, cx);
        })
        .detach();
        let focused = std::rc::Rc::new(std::cell::Cell::new(false));
        window.open_dialog(cx, move |dialog, _, _| {
            let (input, focused) = (input.clone(), focused.clone());
            dialog.title("New Branch").content(move |content, window, cx| {
                if !focused.replace(true) {
                    input.read(cx).focus_handle(cx).focus(window, cx);
                }
                content.child(Input::new(&input))
            })
        });
    }

    pub(super) fn pick_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.root.clone() else { return };
        let branches = git::branches(&root);
        let current = self.branch.as_deref();
        // Row 0 creates a branch; the rest are the existing ones.
        let selected = branches.iter().position(|b| Some(b.as_str()) == current).map(|ix| ix + 1);
        let items = std::iter::once("New Branch…".to_string())
            .chain(branches.iter().cloned())
            .map(|label| CommandItem::new().label(label))
            .collect();
        let this = cx.entity().downgrade();
        palette::open(
            items,
            selected,
            |_, _, _| {},
            move |ix, window, cx| {
                let this = this.clone();
                if ix == 0 {
                    // After this palette closes, or closing it would take the new dialog with it.
                    return window.defer(cx, move |window, cx| {
                        this.update(cx, |this, cx| this.new_branch(window, cx)).ok();
                    });
                }
                // An uncommitted change that conflicts with the branch makes git refuse; show why.
                if let Err(e) = git::switch(&root, &branches[ix - 1]) {
                    window.push_notification(e, cx);
                }
                this.update(cx, |this, cx| {
                    this.load_branch(cx);
                    this.load_status(cx);
                    this.with_git(|git, _, cx| git.refresh(cx), window, cx);
                })
                .ok();
            },
            |_, _| {},
            window,
            cx,
        );
    }

    pub(super) fn file_history(&mut self, _: &FileHistory, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(root), Some(tab)) = (self.root.clone(), self.tabs.get(self.active)) else { return };
        let file = tab.path.clone();
        let commits = cx.background_spawn({
            let file = file.clone();
            async move { git::log(&root, Some(&file)) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let commits = commits.await;
            if commits.is_empty() {
                return;
            }
            let items = commits
                .iter()
                .map(|c| CommandItem::new().label(format!("{}  {}  {} · {}", c.hash, c.subject, c.author, c.when)))
                .collect();
            this.update_in(cx, |_, window, cx| {
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
            })
            .ok();
        })
        .detach();
    }
}
