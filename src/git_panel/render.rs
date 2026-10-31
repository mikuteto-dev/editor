use std::ops::Range;

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Icon, Sizable, Theme,
        button::{Button, ButtonVariants},
        h_flex,
        input::Textarea,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::{Check, CommitChanges, GitEvent, GitPanel, Row, Section};
use crate::icons;

const BRANCH_ICON: &[u8] = include_bytes!("../../assets/ui/git-branch.svg");
const CHECK_ICON: &[u8] = include_bytes!("../../assets/ui/check.svg");
const MINUS_ICON: &[u8] = include_bytes!("../../assets/ui/minus.svg");
const DISCARD_ICON: &[u8] = include_bytes!("../../assets/ui/undo-2.svg");
const FETCH_ICON: &[u8] = include_bytes!("../../assets/ui/refresh-cw.svg");
const PULL_ICON: &[u8] = include_bytes!("../../assets/ui/arrow-down.svg");
const PUSH_ICON: &[u8] = include_bytes!("../../assets/ui/arrow-up.svg");

impl GitPanel {
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
