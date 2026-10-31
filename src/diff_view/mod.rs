mod patch;

use std::{collections::HashMap, rc::Rc};

use gpui_kit::{
    component::{
        ActiveTheme, Selectable, Sizable,
        button::{Button, ButtonVariants},
        diff::{Diff, DiffFile, DiffHoverHighlight, DiffMode, DiffState},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};

pub use patch::{Part, load};

use crate::{git::CommitInfo, icons};

/// Files with more changed lines than this open folded; a click on the header opens them.
const FOLD_LINES: usize = 1_000;

pub struct DiffView {
    title: SharedString,
    meta: Option<SharedString>,
    body: Option<SharedString>,
    parts: Vec<Part>,
    state: Entity<DiffState>,
}

impl DiffView {
    pub fn new(title: SharedString, info: Option<CommitInfo>, parts: Vec<Part>, cx: &mut Context<Self>) -> Self {
        let folded: Vec<SharedString> =
            parts.iter().filter(|p| p.added + p.removed > FOLD_LINES).map(|p| p.path().clone()).collect();
        let files: Vec<DiffFile> = parts.iter().map(|p| p.file.clone()).collect();
        let state = cx.new(|cx| {
            let mut state = DiffState::new(files, cx);
            for path in &folded {
                state.set_file_collapsed(path, true, cx);
            }
            state
        });
        let (meta, body) = match info {
            Some(info) => (
                Some(format!("{} · {} · {}", info.author, info.when, info.hash).into()),
                Some(info.body).filter(|b| !b.is_empty()).map(Into::into),
            ),
            None => (None, None),
        };
        Self { title, meta, body, parts, state }
    }

    fn show_more(&mut self, path: &SharedString, cx: &mut Context<Self>) {
        let Some(part) = self.parts.iter_mut().find(|p| p.path() == path) else { return };
        if !part.show_more() {
            return;
        }
        let files: Vec<DiffFile> = self.parts.iter().map(|p| p.file.clone()).collect();
        // Replacing the files resets the scroll position; come back to the file that grew.
        self.state.update(cx, |state, cx| {
            state.set_files(files, cx);
            state.scroll_to_file(path, cx);
        });
        cx.notify();
    }

    fn set_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_mode(mode, cx));
        cx.notify();
    }
}

impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mode = self.state.read(cx).mode();
        let (added, removed) = self.parts.iter().fold((0, 0), |(a, r), p| (a + p.added, r + p.removed));
        let files = self.parts.len();
        let stats = h_flex()
            .gap_2()
            .text_xs()
            .child(div().text_color(theme.muted_foreground).child(format!("{files} {}", if files == 1 { "file" } else { "files" })))
            .child(div().text_color(theme.green).child(format!("+{added}")))
            .child(div().text_color(theme.red).child(format!("−{removed}")));
        let header = v_flex()
            .flex_none()
            .px_4()
            .py_3()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().min_w_0().truncate().text_base().font_weight(FontWeight::SEMIBOLD).child(self.title.clone()))
                    .child(stats)
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(
                                Button::new("unified")
                                    .label("Unified")
                                    .ghost()
                                    .xsmall()
                                    .selected(mode == DiffMode::Unified)
                                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Unified, cx))),
                            )
                            .child(
                                Button::new("split")
                                    .label("Split")
                                    .ghost()
                                    .xsmall()
                                    .selected(mode == DiffMode::Split)
                                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Split, cx))),
                            ),
                    ),
            )
            .children(self.meta.clone().map(|meta| div().text_xs().text_color(theme.muted_foreground).child(meta)))
            .children(self.body.clone().map(|body| div().pt_1().text_sm().text_color(theme.foreground).child(body)));
        let stats: Rc<HashMap<SharedString, (usize, usize, usize)>> =
            Rc::new(self.parts.iter().map(|p| (p.path().clone(), (p.added, p.removed, p.hidden()))).collect());
        let view = cx.entity().downgrade();
        v_flex().size_full().child(header).child(
            Diff::new(&self.state)
                .line_number(true)
                .syntax_highlight(true)
                .hover_highlight(DiffHoverHighlight::Line)
                .header_prefix(|file, _, _| {
                    let name = file.path().rsplit('/').next().unwrap_or_default().to_string();
                    img(icons::file(&name)).size_4().flex_none()
                })
                .render_header_suffix(move |file, _, cx| {
                    let theme = cx.theme();
                    let (added, removed, hidden) = stats.get(file.path()).copied().unwrap_or_default();
                    let path = file.path().clone();
                    let view = view.clone();
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(div().text_color(theme.green).child(format!("+{added}")))
                        .child(div().text_color(theme.red).child(format!("−{removed}")))
                        .when(hidden > 0, |el| {
                            el.child(
                                Button::new(SharedString::from(format!("more:{path}")))
                                    .label(format!("Show more · {hidden} lines hidden"))
                                    .ghost()
                                    .xsmall()
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |view, cx| view.show_more(&path, cx)).ok();
                                    }),
                            )
                        })
                })
                .flex_1()
                .min_h_0(),
        )
    }
}
