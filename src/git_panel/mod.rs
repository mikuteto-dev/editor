mod render;

use std::path::{Path, PathBuf};

use gpui_kit::{
    component::{WindowExt, input::TextareaState},
    *,
};

pub use render::status_color;

use crate::git::{self, Change, Commit};

actions!(git_panel, [CommitChanges]);

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-enter", CommitChanges, Some("GitPanel"))]);
}

pub enum GitEvent {
    Diff(PathBuf),
    Open(PathBuf),
    Commit(String),
    PickBranch,
    Changed,
}

impl EventEmitter<GitEvent> for GitPanel {}

/// A checkbox's state: a file or section can be partly staged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Check {
    Off,
    Partial,
    On,
}

impl Check {
    fn of(change: &Change) -> Self {
        match (change.staged, change.unstaged) {
            (true, false) => Self::On,
            (true, true) => Self::Partial,
            _ => Self::Off,
        }
    }

    fn of_all<'a>(changes: impl Iterator<Item = &'a Change>) -> Self {
        let checks: Vec<_> = changes.map(Self::of).collect();
        if checks.iter().all(|c| *c == Self::On) {
            Self::On
        } else if checks.iter().all(|c| *c == Self::Off) {
            Self::Off
        } else {
            Self::Partial
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Conflicts,
    Tracked,
    Untracked,
    History,
}

fn section_of(change: &Change) -> Section {
    match change.status {
        'C' => Section::Conflicts,
        'U' => Section::Untracked,
        _ => Section::Tracked,
    }
}

#[derive(Clone, Copy)]
enum Row {
    Header(Section),
    Change(usize),
    Commit(usize),
}

pub struct GitPanel {
    root: PathBuf,
    changes: Vec<Change>,
    commits: Vec<Commit>,
    rows: Vec<Row>,
    branch: Option<String>,
    ahead_behind: Option<(u32, u32)>,
    message: Entity<TextareaState>,
    /// The network operation in flight; the remote buttons wait for it.
    busy: Option<&'static str>,
    amend: bool,
    task: Task<()>,
}

impl GitPanel {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| TextareaState::new(window, cx).placeholder("Commit message").auto_grow(2, 8));
        cx.observe(&message, |_, _, cx| cx.notify()).detach();
        let mut panel = Self {
            root,
            changes: Vec::new(),
            commits: Vec::new(),
            rows: Vec::new(),
            branch: None,
            ahead_behind: None,
            message,
            busy: None,
            amend: false,
            task: Task::ready(()),
        };
        panel.refresh(cx);
        panel
    }

    /// Status comes from the workspace, which shares it with the file tree.
    pub fn set_changes(&mut self, changes: Vec<Change>, cx: &mut Context<Self>) {
        self.changes = changes;
        self.rebuild();
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.task = cx.spawn(async move |this, cx| {
            let (commits, branch, ahead_behind) = cx
                .background_spawn(async move { (git::log(&root, None), git::branch(&root), git::ahead_behind(&root)) })
                .await;
            this.update(cx, |this, cx| {
                this.commits = commits;
                this.branch = branch;
                this.ahead_behind = ahead_behind;
                this.rebuild();
                cx.notify();
            })
            .ok();
        });
    }

    fn rebuild(&mut self) {
        self.rows.clear();
        for section in [Section::Conflicts, Section::Tracked, Section::Untracked] {
            let ixs: Vec<usize> = (0..self.changes.len()).filter(|&ix| section_of(&self.changes[ix]) == section).collect();
            if !ixs.is_empty() {
                self.rows.push(Row::Header(section));
                self.rows.extend(ixs.into_iter().map(Row::Change));
            }
        }
        self.rows.push(Row::Header(Section::History));
        self.rows.extend((0..self.commits.len()).map(Row::Commit));
    }

    fn section(&self, section: Section) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(move |c| section_of(c) == section)
    }

    fn run(
        &mut self,
        busy: Option<&'static str>,
        op: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if busy.is_some() {
            self.busy = busy;
            cx.notify();
        }
        let root = self.root.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { op(&root) }).await;
            this.update_in(cx, |this, window, cx| {
                if busy.is_some() {
                    this.busy = None;
                }
                if let Err(e) = result {
                    window.push_notification(e, cx);
                }
                cx.emit(GitEvent::Changed);
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    pub fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(Some("Fetching…"), git::fetch, window, cx)
    }

    pub fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(Some("Pulling…"), git::pull, window, cx)
    }

    pub fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(Some("Pushing…"), git::push, window, cx)
    }

    pub fn stash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(None, git::stash, window, cx)
    }

    pub fn stash_pop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(None, git::stash_pop, window, cx)
    }

    fn toggle(&mut self, paths: Vec<PathBuf>, check: Check, window: &mut Window, cx: &mut Context<Self>) {
        // A partly staged entry stages the rest, as in Zed.
        match check {
            Check::On => self.run(None, move |root| git::unstage(root, &paths), window, cx),
            _ => self.run(None, move |root| git::stage(root, &paths), window, cx),
        }
    }

    fn discard(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let change = &self.changes[ix];
        let name = change.path.file_name().unwrap_or_default().to_string_lossy();
        let detail = if matches!(change.status, 'U' | 'A') {
            "The file moves to the Trash."
        } else {
            "Changes since the last commit are lost."
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Discard changes to {name}?"),
            Some(detail),
            &["Discard", "Cancel"],
            cx,
        );
        let change = Change { path: change.path.clone(), ..*change };
        cx.spawn_in(window, async move |this, cx| {
            if answer.await.ok()? != 0 {
                return None;
            }
            this.update_in(cx, |this, window, cx| this.run(None, move |root| git::discard(root, &change), window, cx))
                .ok()
        })
        .detach();
    }

    fn commit(&mut self, _: &CommitChanges, window: &mut Window, cx: &mut Context<Self>) {
        let message = self.message.read(cx).value().trim().to_string();
        let Some(tracked) = self.commit_kind() else { return };
        if message.is_empty() {
            return;
        }
        let (root, amend) = (self.root.clone(), self.amend);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_spawn(async move { git::commit(&root, &message, tracked, amend) }).await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(()) => {
                        this.amend = false;
                        this.message.update(cx, |m, cx| m.set_value("", window, cx))
                    }
                    Err(e) => window.push_notification(e, cx),
                }
                cx.emit(GitEvent::Changed);
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    /// `Some(true)` commits every tracked change, `Some(false)` the staged ones.
    fn commit_kind(&self) -> Option<bool> {
        // Amending may only reword the last commit, so it needs no changes.
        if self.amend || self.changes.iter().any(|c| c.staged) {
            Some(false)
        } else if self.changes.iter().any(|c| !c.untracked()) {
            Some(true)
        } else {
            None
        }
    }

    fn toggle_amend(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.amend = !self.amend;
        // Amending starts from the message being replaced.
        if self.amend && self.message.read(cx).value().trim().is_empty() {
            let last = git::last_message(&self.root).unwrap_or_default();
            self.message.update(cx, |m, cx| m.set_value(last, window, cx));
        }
        cx.notify();
    }
}

#[std::prelude::rust_2024::test]
fn sections_check_what_is_staged() {
    let change = |status, staged, unstaged| Change { path: PathBuf::new(), status, staged, unstaged };
    let (on, partial, off) = (change('M', true, false), change('M', true, true), change('U', false, true));
    assert_eq!([Check::of(&on), Check::of(&partial), Check::of(&off)], [Check::On, Check::Partial, Check::Off]);
    assert_eq!(Check::of_all([&on, &on].into_iter()), Check::On);
    assert_eq!(Check::of_all([&on, &off].into_iter()), Check::Partial);
    assert_eq!(Check::of_all([&off].into_iter()), Check::Off);
    assert!(section_of(&change('C', true, true)) == Section::Conflicts);
    assert!(section_of(&off) == Section::Untracked);
}
