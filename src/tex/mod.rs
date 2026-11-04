mod build;
mod pdf;

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui_kit::{
    component::{
        ActiveTheme, Disableable, Selectable, Sizable,
        button::{Button, ButtonGroup, ButtonVariants},
        h_flex,
        input::EditorState,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Lua,
    Xe,
    Platex,
}

impl Engine {
    pub const ALL: [Self; 3] = [Self::Lua, Self::Xe, Self::Platex];

    fn program(self) -> &'static str {
        match self {
            Self::Lua => "lualatex",
            Self::Xe => "xelatex",
            Self::Platex => "platex",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Lua => "LuaLaTeX",
            Self::Xe => "XeLaTeX",
            Self::Platex => "pLaTeX",
        }
    }

    fn mode(self) -> &'static str {
        match self {
            Self::Lua => "-lualatex",
            Self::Xe => "-xelatex",
            Self::Platex => "-pdfdvi",
        }
    }

    fn from_source(source: &str) -> Result<Self, String> {
        for line in source.lines().take(20) {
            let Some(comment) = line.trim_start().strip_prefix('%') else {
                continue;
            };
            let Some((key, value)) = comment.trim().split_once('=') else {
                continue;
            };
            let key = key
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            if key != "!tex program" && key != "!tex ts-program" {
                continue;
            }
            let value = value.trim();
            return Self::ALL
                .into_iter()
                .find(|engine| engine.program().eq_ignore_ascii_case(value))
                .ok_or_else(|| {
                    format!("Unsupported TeX engine: {value}. Select LuaLaTeX, XeLaTeX or pLaTeX.")
                });
        }
        Ok(Self::Lua)
    }
}

pub struct Preview {
    path: PathBuf,
    root: PathBuf,
    editor: Entity<EditorState>,
    engine: Engine,
    bytes: Option<Arc<Vec<u8>>>,
    pages: usize,
    page: usize,
    image: Option<pdf::Page>,
    error: Option<SharedString>,
    busy: bool,
    cancel: Arc<AtomicBool>,
    task: Task<()>,
}

impl Preview {
    pub fn new(
        path: &Path,
        root: &Path,
        editor: Entity<EditorState>,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected = Engine::from_source(&editor.read(cx).text().to_string());
        let mut this = Self {
            path: path.into(),
            root: root.into(),
            editor,
            engine: selected.as_ref().copied().unwrap_or(Engine::Lua),
            bytes: None,
            pages: 0,
            page: 0,
            image: None,
            error: selected.as_ref().err().cloned().map(Into::into),
            busy: false,
            cancel: Arc::new(AtomicBool::new(false)),
            task: Task::ready(()),
        };
        if selected.is_ok() {
            this.build(cx);
        }
        this
    }

    fn stop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.task = Task::ready(());
        self.busy = false;
    }

    fn build(&mut self, cx: &mut Context<Self>) {
        self.stop();
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        let (path, root, engine) = (self.path.clone(), self.root.clone(), self.engine);
        let source = self.editor.read(cx).text().to_string();
        self.busy = true;
        self.error = None;
        let job = cx.background_spawn(async move {
            let bytes = build::compile(&path, &root, &source, engine, &cancel)?;
            let pages = pdf::pages(&bytes)?;
            let image = pdf::render(&bytes, 0)?;
            Ok::<_, String>((Arc::new(bytes), pages, image))
        });
        self.task = cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok((bytes, pages, image)) => {
                        this.bytes = Some(bytes);
                        this.pages = pages;
                        this.page = 0;
                        this.image = Some(image);
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        });
        cx.notify();
    }

    fn show(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.busy || page >= self.pages {
            return;
        }
        let Some(bytes) = self.bytes.clone() else {
            return;
        };
        self.busy = true;
        self.error = None;
        let job = cx.background_spawn(async move { pdf::render(&bytes, page) });
        self.task = cx.spawn(async move |this, cx| {
            let result = job.await;
            _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(image) => {
                        this.image = Some(image);
                        this.page = page;
                    }
                    Err(error) => this.error = Some(error.into()),
                }
                cx.notify();
            });
        });
        cx.notify();
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engines =
            ButtonGroup::new("tex-engine")
                .small()
                .compact()
                .children(Engine::ALL.into_iter().map(|engine| {
                    Button::new(engine.program())
                        .label(engine.label())
                        .selected(self.engine == engine)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.engine == engine {
                                return;
                            }
                            this.engine = engine;
                            this.bytes = None;
                            this.image = None;
                            this.pages = 0;
                            this.page = 0;
                            this.build(cx);
                        }))
                }));
        let toolbar =
            h_flex()
                .flex_wrap()
                .gap_3()
                .px_4()
                .py_2()
                .child(engines)
                .child(
                    Button::new("tex-build")
                        .small()
                        .label(if self.busy { "Cancel" } else { "Build" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.busy {
                                this.stop();
                                cx.notify();
                            } else {
                                this.build(cx);
                            }
                        })),
                )
                .child(div().flex_1())
                .when(self.pages > 0, |bar| {
                    bar.child(
                        Button::new("pdf-prev")
                            .small()
                            .ghost()
                            .label("←")
                            .tooltip("Previous page")
                            .disabled(self.busy || self.page == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show(this.page.saturating_sub(1), cx)
                            })),
                    )
                    .child(format!("{} / {}", self.page + 1, self.pages))
                    .child(
                        Button::new("pdf-next")
                            .small()
                            .ghost()
                            .label("→")
                            .tooltip("Next page")
                            .disabled(self.busy || self.page + 1 >= self.pages)
                            .on_click(cx.listener(|this, _, _, cx| this.show(this.page + 1, cx))),
                    )
                });
        let error = self.error.clone().map(|error| {
            div()
                .id("tex-error")
                .max_h(px(200.))
                .overflow_y_scroll()
                .px_4()
                .py_2()
                .text_sm()
                .text_color(cx.theme().red)
                .child(error)
        });
        let image = self.image.as_ref().map(|page| {
            img(page.image.clone())
                .w_full()
                .max_w(px(960.))
                .aspect_ratio(page.width as f32 / page.height as f32)
        });
        v_flex()
            .size_full()
            .min_w_0()
            .child(toolbar)
            .children(error)
            .child(
                div()
                    .id("tex-pdf")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_4()
                    .flex()
                    .flex_col()
                    .items_center()
                    .children(image),
            )
    }
}

#[cfg(test)]
mod tests;
