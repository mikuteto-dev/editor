use gpui_kit::{
    component::{
        ActiveTheme, Icon, IconName, Sizable, TitleBar,
        button::{Button, ButtonVariants},
        h_flex,
        kbd::Kbd,
        resizable::{h_resizable, resizable_panel},
        tab::{Tab as TabItem, TabBar},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::{Panel, Workspace};
use crate::{
    commands::*,
    git_panel::GitPanel,
    icons,
    licenses::Verdict,
    tab::{Body, Tab, render_body},
    theme::{self, SelectTheme},
};

const BRANCH_ICON: &[u8] = include_bytes!("../../assets/ui/git-branch.svg");
const EYE_ICON: &[u8] = include_bytes!("../../assets/ui/eye.svg");
const CODE_ICON: &[u8] = include_bytes!("../../assets/ui/code.svg");

impl Workspace {
    fn render_tabs(&self, cx: &mut Context<Self>) -> TabBar {
        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let suffix = if tab.dirty() {
                div().size_2().mx_1().rounded_full().bg(cx.theme().foreground).into_any_element()
            } else {
                Button::new(ix)
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close(ix, window, cx);
                    }))
                    .into_any_element()
            };
            TabItem::new()
                .label(tab.name.clone())
                .when(!tab.pinned, |item| item.italic())
                .prefix(img(icons::file(&tab.name)).size_4().ml_2())
                .suffix(suffix)
        });
        let preview = self.tabs.get(self.active).and_then(|tab| match &tab.body {
            Body::Text(text) if text.previewable.is_some() => Some((EYE_ICON, "Open Preview")),
            Body::Preview(_) => Some((CODE_ICON, "Show Code")),
            _ => None,
        });
        let preview = preview.map(|(svg, tip)| {
            Button::new("preview")
                .icon(Icon::default().data(svg))
                .ghost()
                .small()
                .tooltip(tip)
                .on_click(cx.listener(|this, _, window, cx| this.toggle_preview(&TogglePreview, window, cx)))
        });
        TabBar::new("tabs")
            .selected_index(self.active)
            .on_click(cx.listener(|this, ix: &usize, window, cx| this.activate(*ix, window, cx)))
            .children(tabs)
            .when_some(preview, |bar, button| bar.suffix(h_flex().h_full().px_1().child(button)))
    }

    fn render_activity(&self, cx: &App) -> Div {
        let theme = cx.theme();
        let items: [(Panel, &'static [u8], Box<dyn Action>); 3] = [
            (Panel::Explorer, include_bytes!("../../assets/ui/files.svg"), Box::new(ShowExplorer)),
            (Panel::Search, include_bytes!("../../assets/ui/search.svg"), Box::new(ShowSearch)),
            (Panel::Git, BRANCH_ICON, Box::new(ShowGit)),
        ];
        let buttons = items.into_iter().map(|(panel, svg, action)| {
            let active = self.sidebar && self.panel == panel;
            let color = if active { theme.foreground } else { theme.muted_foreground };
            let badge = (panel == Panel::Git && self.change_count > 0).then(|| {
                div()
                    .absolute()
                    .bottom(px(3.))
                    .right(px(1.))
                    .min_w(px(15.))
                    .h(px(15.))
                    .px(px(3.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .text_size(px(9.))
                    .child(self.change_count.min(99).to_string())
            });
            div()
                .id(panel as usize)
                .relative()
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(theme.radius)
                .hover(|el| el.bg(theme.sidebar_accent))
                .child(Icon::default().data(svg).size_5().text_color(color))
                .children(badge)
                .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
        });
        v_flex()
            .w(px(44.))
            .pt_1()
            .gap_1()
            .items_center()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .children(buttons)
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = (!self.tabs.is_empty()).then(|| self.render_tabs(cx));
        let start = self.tabs.is_empty().then(|| render_start(window, cx));
        let video = match self.tabs.get_mut(self.active).map(|t| &mut t.body) {
            Some(Body::Video(video)) => Some(
                div()
                    .id("video")
                    .size_full()
                    .p_6()
                    .children(video.render(window))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_play(&TogglePlay, window, cx))),
            ),
            _ => None,
        };
        let theme = cx.theme();
        let tab = self.tabs.get(self.active);
        let title = self.root.as_ref().and_then(|r| r.file_name()).unwrap_or_default();
        let title = title.to_string_lossy().into_owned();
        let side: Option<AnyView> = match self.panel {
            Panel::Explorer => self.tree.as_ref().map(|(view, _)| view.clone().into()),
            Panel::Search => self.search.as_ref().map(|(view, _)| view.clone().into()),
            Panel::Git => self.git.as_ref().map(|(view, _)| view.clone().into()),
        };
        let activity = side.is_some().then(|| self.render_activity(cx));
        let side = side.filter(|_| self.sidebar);
        let sync = match self.ahead_behind {
            Some((ahead, behind)) if ahead + behind > 0 => format!("↑{ahead} ↓{behind}"),
            _ => String::new(),
        };
        let branch = self.branch.clone().map(|branch| {
            h_flex()
                .id("branch")
                .gap_1()
                .px_1()
                .rounded_sm()
                .hover(|el| el.bg(theme.secondary_hover))
                .child(Icon::default().data(BRANCH_ICON).size_3p5())
                .child(branch)
                .child(sync)
                .on_click(cx.listener(|this, _, window, cx| this.pick_branch(window, cx)))
        });
        let position = tab.and_then(Tab::text).map(|text| {
            let pos = text.editor.read(cx).cursor_position();
            // The caret on a dependency line names its license, no mouse needed; elsewhere, who last changed the line.
            let note = match text.licenses.borrow().get(&pos.line) {
                Some(line) => {
                    let color = match line.verdict {
                        Verdict::Ok => theme.muted_foreground,
                        Verdict::Caution(_) | Verdict::Unknown(_) => theme.warning,
                        Verdict::Conflict(_) => theme.danger,
                    };
                    Some(div().text_color(color).child(line.summary.clone()))
                }
                None => text
                    .blame
                    .get(pos.line as usize)
                    .cloned()
                    .flatten()
                    .map(|blame| div().max_w(px(480.)).truncate().text_color(theme.muted_foreground).child(blame)),
            };
            h_flex()
                .gap_4()
                .children(note)
                .child(text.language)
                .child(format!("Ln {}, Col {}", pos.line + 1, pos.character + 1))
        });
        let body = tab.filter(|_| video.is_none()).map(|tab| render_body(tab, cx));
        let video_in_front = video.is_some();

        v_flex()
            .size_full()
            .track_focus(&self.focus)
            // Space plays and pauses only while a video is in front; elsewhere it types.
            .when(video_in_front, |el| el.key_context("Video"))
            .bg(theme.background)
            .text_color(theme.foreground)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::open_recent))
            .on_action(cx.listener(Self::command_palette))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::toggle_play))
            .on_action(cx.listener(Self::file_history))
            .on_action(cx.listener(Self::show_licenses))
            .on_action(cx.listener(Self::show_graph))
            .on_action(cx.listener(|this, _: &SwitchBranch, window, cx| this.pick_branch(window, cx)))
            .on_action(cx.listener(|this, _: &NewBranch, window, cx| this.new_branch(window, cx)))
            .on_action(cx.listener(|this, _: &StashChanges, window, cx| this.with_git(GitPanel::stash, window, cx)))
            .on_action(cx.listener(|this, _: &PopStash, window, cx| this.with_git(GitPanel::stash_pop, window, cx)))
            .on_action(cx.listener(|this, _: &GitFetch, window, cx| this.with_git(GitPanel::fetch, window, cx)))
            .on_action(cx.listener(|this, _: &GitPull, window, cx| this.with_git(GitPanel::pull, window, cx)))
            .on_action(cx.listener(|this, _: &GitPush, window, cx| this.with_git(GitPanel::push, window, cx)))
            .on_action(|_: &SelectTheme, window, cx| theme::open_picker(window, cx))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &PrevTab, window, cx| this.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.sidebar = !this.sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ShowExplorer, window, cx| this.show(Panel::Explorer, window, cx)))
            .on_action(cx.listener(|this, _: &ShowSearch, window, cx| this.show(Panel::Search, window, cx)))
            .on_action(cx.listener(|this, _: &ShowGit, window, cx| this.show(Panel::Git, window, cx)))
            .child(
                TitleBar::new().child(
                    div()
                        .flex_1()
                        .flex()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(title),
                ),
            )
            .child(h_flex().flex_1().min_h_0().items_stretch().children(activity).child(
                // Keyed by panel count: the group keeps stale sizes when a panel appears later.
                h_resizable(if side.is_some() { "main-side" } else { "main" })
                    .when_some(side, |group, side| {
                        group.child(
                            resizable_panel()
                                .size(px(240.))
                                .size_range(px(160.)..px(480.))
                                .child(side),
                        )
                    })
                    .child(
                        resizable_panel().child(
                            v_flex()
                                .size_full()
                                .children(start)
                                .children(tabs)
                                .children(body)
                                .children(video),
                        ),
                    ),
            ))
            .child(
                h_flex()
                    .h(px(24.))
                    .px_3()
                    .justify_between()
                    .text_xs()
                    .bg(theme.status_bar)
                    .text_color(theme.muted_foreground)
                    .child(div().children(branch))
                    .children(position),
            )
    }
}

fn render_start(window: &Window, cx: &App) -> Div {
    let theme = cx.theme();
    let hover = theme.secondary;
    let commands: [(&str, Box<dyn Action>); 3] = [
        ("Open Folder", Box::new(OpenFolder)),
        ("Open Recent", Box::new(OpenRecent)),
        ("Command Palette", Box::new(CommandPalette)),
    ];
    let rows = commands.into_iter().enumerate().map(|(ix, (label, action))| {
        let kbd = Kbd::global_binding_for_action(action.as_ref(), window).map(Kbd::outline);
        h_flex()
            .id(ix)
            .h(px(32.))
            .px_3()
            .gap_8()
            .justify_between()
            .rounded(theme.radius)
            .hover(move |el| el.bg(hover))
            .child(label)
            .children(kbd)
            .on_click(move |_, window, cx| window.dispatch_action(action.boxed_clone(), cx))
    });
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .child(v_flex().w(px(280.)).gap_1().text_sm().text_color(theme.muted_foreground).children(rows))
}
