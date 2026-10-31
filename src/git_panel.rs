use std::{ops::Range, path::PathBuf};

use gpui_kit::{
    component::{ActiveTheme, Theme, h_flex},
    *,
};

use crate::{
    git::{self, Change, Commit},
    icons,
};

pub enum GitEvent {
    Diff(PathBuf),
    Commit(String),
}

impl EventEmitter<GitEvent> for GitPanel {}

pub struct GitPanel {
    root: PathBuf,
    changes: Vec<Change>,
    commits: Vec<Commit>,
    task: Task<()>,
}

impl GitPanel {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut panel = Self {
            root,
            changes: Vec::new(),
            commits: Vec::new(),
            task: Task::ready(()),
        };
        panel.refresh(cx);
        panel
    }

    /// Status comes from the workspace, which shares it with the file tree.
    pub fn set_changes(&mut self, changes: Vec<Change>, cx: &mut Context<Self>) {
        self.changes = changes;
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        self.task = cx.spawn(async move |this, cx| {
            let commits = cx.background_spawn(async move { git::log(&root, None) }).await;
            this.update(cx, |this, cx| {
                this.commits = commits;
                cx.notify();
            })
            .ok();
        });
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        let hover = theme.sidebar_accent;
        let header = |ix: usize, label: &str, count: usize| {
            // uniform_list needs every row at the same height.
            h_flex()
                .id(ix)
                .h(px(24.))
                .px_2()
                .gap_2()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label.to_uppercase())
                .child(count.to_string())
        };
        let changes_end = 1 + self.changes.len();
        range
            .map(|ix| {
                let row = h_flex().id(ix).h(px(24.)).w_full().px_2().gap_1p5().rounded_sm();
                if ix == 0 {
                    return header(ix, "Changes", self.changes.len());
                }
                if ix == changes_end {
                    return header(ix, "History", self.commits.len());
                }
                if ix < changes_end {
                    let change = &self.changes[ix - 1];
                    let name = change.path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    let dir = change.path.parent().and_then(|p| p.strip_prefix(&self.root).ok());
                    let dir = dir.map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                    let color = status_color(change.status, theme);
                    let path = change.path.clone();
                    return row
                        .hover(move |el| el.bg(hover))
                        .child(img(icons::file(&name)).size_4().flex_none())
                        .child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1p5()
                                .child(div().flex_none().text_color(color).child(name))
                                .child(div().truncate().text_xs().text_color(theme.muted_foreground).child(dir)),
                        )
                        .child(div().text_xs().text_color(color).child(change.status.to_string()))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(GitEvent::Diff(path.clone()))));
                }
                let commit = &self.commits[ix - changes_end - 1];
                let hash = commit.hash.clone();
                row.hover(move |el| el.bg(hover))
                    .child(div().flex_1().truncate().child(commit.subject.clone()))
                    .child(div().flex_none().text_xs().text_color(theme.muted_foreground).child(commit.when.clone()))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(GitEvent::Commit(hash.clone()))))
            })
            .collect()
    }
}

pub fn status_color(status: char, theme: &Theme) -> Hsla {
    match status {
        'A' | 'U' => theme.green,
        'D' => theme.red,
        _ => theme.yellow,
    }
}

impl Render for GitPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        uniform_list(
            "git",
            2 + self.changes.len() + self.commits.len(),
            cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
        )
        .size_full()
        .p_1()
        .text_sm()
        .bg(theme.sidebar)
        .text_color(theme.sidebar_foreground)
    }
}
