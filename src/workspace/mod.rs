mod git;
mod licenses;
mod pickers;
mod render;
mod sync;
mod tabs;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::{component::WindowExt, *};

use crate::{
    commands::Quit,
    git_panel::{GitEvent, GitPanel},
    licenses::Report,
    recent,
    search::{OpenAt, SearchPanel},
    session::{self, Session},
    tab::{Body, Tab},
    tree::{FileTree, TreeEvent},
    watch::Watch,
};

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Explorer,
    Search,
    Git,
}

pub struct Workspace {
    root: Option<PathBuf>,
    tree: Option<(Entity<FileTree>, Subscription)>,
    search: Option<(Entity<SearchPanel>, Subscription)>,
    git: Option<(Entity<GitPanel>, Subscription)>,
    branch: Option<SharedString>,
    change_count: usize,
    ahead_behind: Option<(u32, u32)>,
    status: Task<()>,
    /// Dependency reports by manifest; `None` while one is being read.
    reports: HashMap<PathBuf, Option<Arc<Report>>>,
    tabs: Vec<Tab>,
    active: usize,
    sidebar: bool,
    panel: Panel,
    focus: FocusHandle,
    _watch: Task<()>,
}

impl Workspace {
    pub fn new(root: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            this.update(cx, |this, cx| this.quit(&Quit, window, cx)).ok();
            false
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = Self {
            root: None,
            tree: None,
            search: None,
            git: None,
            branch: None,
            change_count: 0,
            ahead_behind: None,
            status: Task::ready(()),
            reports: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            sidebar: true,
            panel: Panel::Explorer,
            focus,
            _watch: Task::ready(()),
        };
        if let Some(root) = root {
            this.set_root(root, window, cx);
        }
        this
    }

    /// A file only picks the folder when none is open yet.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if path.is_dir() {
            return self.switch(path, window, cx);
        }
        if self.root.is_none()
            && let Some(dir) = path.parent()
        {
            self.set_root(dir.to_path_buf(), window, cx);
        }
        self.open(path, true, window, cx);
    }

    fn switch(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard(window, cx, move |this, window, cx| {
            this.save_session(window, cx);
            this.tabs.clear();
            this.set_root(root, window, cx);
        });
    }

    fn save_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = &self.root else { return };
        let active = self.tabs.get(self.active).map(|t| &t.path);
        // Diff and commit tabs have no file behind them.
        let tabs: Vec<_> = self
            .tabs
            .iter()
            .filter(|tab| !matches!(tab.body, Body::Preview(_)) && tab.path.is_file())
            .map(|tab| tab.path.clone())
            .collect();
        let active = tabs.iter().position(|p| Some(p) == active).unwrap_or(0);
        if let Err(e) = session::save(root, &Session { tabs, active }) {
            window.push_notification(format!("session: {e}"), cx);
        }
    }

    fn restore(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Session { tabs, active }) = session::load(root) else {
            return self.activate(0, window, cx);
        };
        let active = tabs.get(active).cloned();
        for path in tabs.into_iter().filter(|p| p.is_file()) {
            self.open(path, true, window, cx);
        }
        let ix = self.tabs.iter().position(|t| Some(&t.path) == active.as_ref());
        self.activate(ix.unwrap_or(0), window, cx);
    }

    fn set_root(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = recent::push(&root) {
            window.push_notification(format!("recent: {e}"), cx);
        }

        let tree = cx.new(|cx| FileTree::new(root.clone(), cx));
        let opened = cx.subscribe_in(&tree, window, |this, _, event, window, cx| match event {
            TreeEvent::Open { path, pin } => this.open(path.clone(), *pin, window, cx),
            TreeEvent::Renamed { from, to } => this.moved(from, to, window, cx),
            TreeEvent::Deleted(path) => this.deleted(path, window, cx),
        });
        self.tree = Some((tree, opened));

        let search = cx.new(|cx| SearchPanel::new(root.clone(), window, cx));
        let opened = cx.subscribe_in(&search, window, |this, _, OpenAt(path, line), window, cx| {
            this.open_at(path.clone(), *line, window, cx)
        });
        self.search = Some((search, opened));

        let git = cx.new(|cx| GitPanel::new(root.clone(), window, cx));
        let picked = cx.subscribe_in(&git, window, |this, _, event, window, cx| match event {
            GitEvent::Diff(path) => this.open_diff(path.clone(), window, cx),
            GitEvent::Open(path) => this.open(path.clone(), true, window, cx),
            GitEvent::Commit(hash) => this.open_commit(hash.clone(), None, window, cx),
            GitEvent::PickBranch => this.pick_branch(window, cx),
            GitEvent::Changed => {
                this.load_status(cx);
                this.load_branch(cx);
            }
        });
        self.git = Some((git, picked));

        self.reports.clear();
        self._watch = self.watch(&root, window, cx);
        self.root = Some(root.clone());
        self.load_branch(cx);
        self.load_status(cx);
        self.restore(&root, window, cx);
    }

    fn watch(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) -> Task<()> {
        match Watch::new(root) {
            Ok(watch) => cx.spawn_in(window, async move |this, cx| {
                while let Some(paths) = watch.next().await {
                    if this.update_in(cx, |this, window, cx| this.changed(paths, window, cx)).is_err() {
                        break;
                    }
                }
            }),
            Err(e) => {
                window.push_notification(format!("watch: {e}"), cx);
                Task::ready(())
            }
        }
    }

    fn show(&mut self, panel: Panel, window: &mut Window, cx: &mut Context<Self>) {
        self.panel = panel;
        self.sidebar = true;
        match panel {
            Panel::Explorer => {
                if let Some((tree, _)) = &self.tree {
                    tree.update(cx, |tree, cx| tree.focus(window, cx));
                }
            }
            Panel::Search => {
                if let Some((search, _)) = &self.search {
                    search.update(cx, |search, cx| search.focus(window, cx));
                }
            }
            // The panel only follows changes while visible; catch up when it appears.
            Panel::Git => {
                if let Some((git, _)) = &self.git {
                    git.update(cx, |git, cx| git.refresh(cx));
                }
            }
        }
        cx.notify();
    }
}
