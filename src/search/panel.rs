use std::{ops::Range, path::PathBuf, time::Duration};

use gpui_kit::{
    component::{
        ActiveTheme, h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    *,
};

use super::{Hit, search};
use crate::icons;

pub struct OpenAt(pub PathBuf, pub u32);

impl EventEmitter<OpenAt> for SearchPanel {}

pub struct SearchPanel {
    root: PathBuf,
    input: Entity<InputState>,
    hits: Vec<Hit>,
    rows: Vec<Row>,
    task: Task<()>,
    _input: Subscription,
}

enum Row {
    File(usize),
    Hit(usize),
}

impl SearchPanel {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let _input = cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
            if let InputEvent::Change = e {
                this.query(cx);
            }
        });
        Self {
            root,
            input,
            hits: Vec::new(),
            rows: Vec::new(),
            task: Task::ready(()),
            _input,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn query(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        let root = self.root.clone();
        // Replacing the task drops the previous one, so only the last keystroke searches.
        self.task = cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_millis(150)).await;
            let hits = match query.is_empty() {
                true => Vec::new(),
                false => cx.background_spawn(async move { search(&root, &query) }).await,
            };
            this.update(cx, |this, cx| {
                this.rows = hits
                    .iter()
                    .enumerate()
                    .flat_map(|(ix, hit)| {
                        let first = ix == 0 || hits[ix - 1].path != hit.path;
                        first.then_some(Row::File(ix)).into_iter().chain([Row::Hit(ix)])
                    })
                    .collect();
                this.hits = hits;
                cx.notify();
            })
            .ok();
        });
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        let hover = theme.sidebar_accent;
        let mark = HighlightStyle {
            background_color: Some(theme.selection),
            ..Default::default()
        };
        range
            .map(|ix| {
                let row = h_flex().id(ix).h(px(24.)).w_full().px_2().gap_1p5().rounded_sm();
                match self.rows[ix] {
                    Row::File(hit) => {
                        let path = &self.hits[hit].path;
                        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        let dir = path
                            .parent()
                            .and_then(|d| d.strip_prefix(&self.root).ok())
                            .map(|d| d.display().to_string())
                            .unwrap_or_default();
                        row.child(img(icons::file(&name)).size_4().flex_none())
                            .child(div().flex_none().child(name))
                            .child(div().truncate().text_xs().text_color(theme.muted_foreground).child(dir))
                    }
                    Row::Hit(hit) => {
                        let Hit { path, line, text, range } = &self.hits[hit];
                        let (path, line) = (path.clone(), *line);
                        row.pl(px(30.))
                            .hover(move |el| el.bg(hover))
                            .child(
                                div()
                                    .truncate()
                                    .child(StyledText::new(text.clone()).with_highlights([(range.clone(), mark)])),
                            )
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenAt(path.clone(), line))))
                    }
                }
            })
            .collect()
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .text_sm()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(div().p_2().child(Input::new(&self.input)))
            .child(
                uniform_list(
                    "hits",
                    self.rows.len(),
                    cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                )
                .flex_1()
                .px_1(),
            )
    }
}
