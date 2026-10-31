use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Theme, WindowExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Textarea, TextareaState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use crate::{
    git::{self, Change, Commit},
    icons,
};

actions!(git_panel, [CommitChanges]);

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-enter", CommitChanges, Some("GitPanel"))]);
}

const BRANCH_ICON: &[u8] = include_bytes!("../assets/ui/git-branch.svg");
const CHECK_ICON: &[u8] = include_bytes!("../assets/ui/check.svg");
const MINUS_ICON: &[u8] = include_bytes!("../assets/ui/minus.svg");
const DISCARD_ICON: &[u8] = include_bytes!("../assets/ui/undo-2.svg");
const FETCH_ICON: &[u8] = include_bytes!("../assets/ui/refresh-cw.svg");
const PULL_ICON: &[u8] = include_bytes!("../assets/ui/arrow-down.svg");
const PUSH_ICON: &[u8] = include_bytes!("../assets/ui/arrow-up.svg");

pub enum GitEvent {
    Diff(PathBuf),
    Open(PathBuf),
    Commit(String),
    PickBranch,
    /// The repository changed through the panel; status needs a reload.
    Changed,
}

impl EventEmitter<GitEvent> for GitPanel {}

#[derive(Clone, Copy, PartialEq)]
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

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (ahead, behind) = self.ahead_behind.unwrap_or_default();
        let remote = |id: &'static str, svg: &'static [u8], count: u32, tip: &'static str| {
            Button::new(id)
                .icon(Icon::default().data(svg))
                .when(count > 0, |b| b.label(count.to_string()))
                .ghost()
                .xsmall()
                .tooltip(tip)
                .disabled(self.busy.is_some())
        };
        h_flex()
            .h(px(32.))
            .flex_none()
            .px_1()
            .gap_0p5()
            .border_b_1()
            .border_color(theme.sidebar_border)
            .child(
                Button::new("branch")
                    .icon(Icon::default().data(BRANCH_ICON))
                    .label(self.branch.clone().unwrap_or_default())
                    .ghost()
                    .xsmall()
                    .tooltip("Switch Branch")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GitEvent::PickBranch))),
            )
            .child(div().flex_1())
            .children(self.busy.map(|label| div().text_xs().text_color(theme.muted_foreground).child(label)))
            .child(remote("fetch", FETCH_ICON, 0, "Fetch").on_click(cx.listener(|this, _, window, cx| this.fetch(window, cx))))
            .child(remote("pull", PULL_ICON, behind, "Pull").on_click(cx.listener(|this, _, window, cx| this.pull(window, cx))))
            .child(remote("push", PUSH_ICON, ahead, "Push").on_click(cx.listener(|this, _, window, cx| this.push(window, cx))))
    }

    fn render_commit_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let kind = self.commit_kind();
        let staged = self.changes.iter().filter(|c| c.staged).count();
        let label = match kind {
            _ if self.amend => "Amend Last Commit".to_string(),
            Some(false) => format!("Commit {staged} {}", if staged == 1 { "File" } else { "Files" }),
            Some(true) => "Commit Tracked".to_string(),
            None => "Commit".to_string(),
        };
        let empty = self.message.read(cx).value().trim().is_empty();
        let amend = if self.amend { Check::On } else { Check::Off };
        v_flex()
            .flex_none()
            .p_2()
            .gap_2()
            .border_t_1()
            .border_color(theme.sidebar_border)
            .child(Textarea::new(&self.message))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .id("amend")
                            .gap_1p5()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(checkbox("amend-box", amend, theme))
                            .child("Amend")
                            .on_click(cx.listener(|this, _, window, cx| this.toggle_amend(window, cx))),
                    )
                    .child(
                        Button::new("commit")
                            .label(label)
                            .primary()
                            .small()
                            .flex_1()
                            .disabled(kind.is_none() || empty)
                            .tooltip_with_action("Commit", &CommitChanges, Some("GitPanel"))
                            .on_click(cx.listener(|this, _, window, cx| this.commit(&CommitChanges, window, cx))),
                    ),
            )
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

    pub fn stash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(None, git::stash, window, cx)
    }

    pub fn stash_pop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(None, git::stash_pop, window, cx)
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = &cx.theme().clone();
        let hover = theme.sidebar_accent;
        range
            .map(|ix| {
                // uniform_list needs every row at the same height.
                let row = h_flex().id(ix).h(px(24.)).w_full().px_2().gap_1p5().rounded_sm();
                match self.rows[ix] {
                    Row::Header(section) => {
                        let (label, count) = match section {
                            // Staging a conflicted file marks it resolved.
                            Section::Conflicts => ("Conflicts", self.section(section).count()),
                            Section::Tracked => ("Tracked", self.section(section).count()),
                            Section::Untracked => ("Untracked", self.section(section).count()),
                            Section::History => ("History", self.commits.len()),
                        };
                        let check = (section != Section::History).then(|| {
                            let check = Check::of_all(self.section(section));
                            let paths: Vec<_> = self.section(section).map(|c| c.path.clone()).collect();
                            checkbox(("section", ix), check, theme).on_click(cx.listener(move |this, _, window, cx| {
                                this.toggle(paths.clone(), check, window, cx)
                            }))
                        });
                        row.text_xs()
                            .text_color(theme.muted_foreground)
                            .children(check)
                            .child(label.to_uppercase())
                            .child(count.to_string())
                    }
                    Row::Change(i) => self.render_change(row, i, cx),
                    Row::Commit(i) => {
                        let commit = &self.commits[i];
                        let hash = commit.hash.clone();
                        row.hover(move |el| el.bg(hover))
                            .child(div().flex_1().truncate().child(commit.subject.clone()))
                            .child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{} · {}", commit.author, commit.when)),
                            )
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(GitEvent::Commit(hash.clone()))))
                    }
                }
            })
            .collect()
    }

    fn render_change(&self, row: Stateful<Div>, ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let hover = theme.sidebar_accent;
        let change = &self.changes[ix];
        let name = change.path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let dir = change.path.parent().and_then(|p| p.strip_prefix(&self.root).ok());
        let dir = dir.map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
        let color = status_color(change.status, theme);
        let check = Check::of(change);
        let path = change.path.clone();
        let paths = vec![path.clone()];
        row.group("change")
            .hover(move |el| el.bg(hover))
            .child(checkbox(("check", ix), check, theme).on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.toggle(paths.clone(), check, window, cx)
            })))
            .child(img(icons::file(&name)).size_4().flex_none())
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .child(div().flex_none().text_color(color).when(change.status == 'D', |el| el.line_through()).child(name))
                    .child(div().truncate().text_xs().text_color(theme.muted_foreground).child(dir)),
            )
            .child(
                Button::new(("discard", ix))
                    .icon(Icon::default().data(DISCARD_ICON))
                    .ghost()
                    .xsmall()
                    .tooltip("Discard Changes")
                    .invisible()
                    .group_hover("change", |style| style.visible())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.discard(ix, window, cx)
                    })),
            )
            .child(div().w_3().flex_none().text_xs().text_color(color).child(change.status.to_string()))
            .on_click(cx.listener(move |_, event: &ClickEvent, _, cx| {
                cx.emit(match event.click_count() {
                    1 => GitEvent::Diff(path.clone()),
                    _ => GitEvent::Open(path.clone()),
                })
            }))
    }
}

fn section_of(change: &Change) -> Section {
    match change.status {
        'C' => Section::Conflicts,
        'U' => Section::Untracked,
        _ => Section::Tracked,
    }
}

fn checkbox(id: impl Into<ElementId>, check: Check, theme: &Theme) -> Stateful<Div> {
    let on = check != Check::Off;
    let mark = match check {
        Check::On => Some(CHECK_ICON),
        Check::Partial => Some(MINUS_ICON),
        Check::Off => None,
    };
    div()
        .id(id)
        .size(px(14.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .border_1()
        .border_color(if on { theme.primary } else { theme.muted_foreground })
        .when(on, |el| el.bg(theme.primary))
        .children(mark.map(|svg| Icon::default().data(svg).size_3().text_color(theme.primary_foreground)))
}

pub fn status_color(status: char, theme: &Theme) -> Hsla {
    match status {
        'A' | 'U' => theme.green,
        'D' | 'C' => theme.red,
        _ => theme.yellow,
    }
}

impl Render for GitPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (bg, fg) = (theme.sidebar, theme.sidebar_foreground);
        v_flex()
            .size_full()
            .key_context("GitPanel")
            .on_action(cx.listener(Self::commit))
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .child(self.render_toolbar(cx))
            .child(
                uniform_list("git", self.rows.len(), cx.processor(|this, range, _, cx| this.render_rows(range, cx)))
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .p_1(),
            )
            .child(self.render_commit_box(cx))
    }
}
