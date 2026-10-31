use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use gpui_kit::{
    component::{
        ActiveTheme,
        highlighter::Language,
        input::{Editor, EditorState, RangeDecorationCollection, Rope},
    },
    *,
};

use crate::video::Video;

pub struct Tab {
    /// Identifies the tab; diffs and commits use a synthetic one.
    pub path: PathBuf,
    pub name: SharedString,
    pub body: Body,
    /// An unpinned tab is a preview: the next single-click open reuses it.
    pub pinned: bool,
}

pub enum Body {
    Text(Text),
    Image,
    Video(Video),
    Graph(Entity<crate::dep_graph::DepGraph>),
    Diff(Entity<crate::diff_view::DiffView>),
}

pub struct Text {
    pub editor: Entity<EditorState>,
    pub language: &'static str,
    /// What is on disk, so undoing back to it clears `dirty`.
    pub saved: Rope,
    pub dirty: bool,
    pub readonly: bool,
    pub previewable: Option<PreviewKind>,
    // Rendered when toggled on, not on every keystroke.
    pub preview: Option<Preview>,
    pub git_marks: Option<RangeDecorationCollection>,
    pub blame: Vec<Option<SharedString>>,
    pub licenses: crate::licenses::Lines,
    pub license_labels: Option<RangeDecorationCollection>,
    pub _subscription: Subscription,
}

#[derive(Clone, Copy)]
pub enum PreviewKind {
    Svg,
    Markdown,
}

pub enum Preview {
    Svg(Arc<Image>),
    Markdown(SharedString),
}

impl PreviewKind {
    pub fn of(path: &Path) -> Option<Self> {
        if has_ext(path, &["svg"]) {
            Some(Self::Svg)
        } else if has_ext(path, &["md", "mdx", "markdown"]) {
            Some(Self::Markdown)
        } else {
            None
        }
    }

    pub fn render(self, text: &str) -> Preview {
        match self {
            Self::Svg => Preview::Svg(Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                text.as_bytes().to_vec(),
            ))),
            Self::Markdown => Preview::Markdown(text.to_string().into()),
        }
    }
}

impl Tab {
    pub fn text(&self) -> Option<&Text> {
        match &self.body {
            Body::Text(text) => Some(text),
            Body::Image | Body::Video(_) | Body::Graph(_) | Body::Diff(_) => None,
        }
    }

    pub fn dirty(&self) -> bool {
        self.text().is_some_and(|t| t.dirty)
    }
}

pub fn render_body(tab: &Tab, cx: &App) -> Div {
    let theme = cx.theme();
    let view = |image: ImageSource| {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_6()
            .flex()
            .items_center()
            .justify_center()
            .child(img(image).max_w_full().max_h_full().object_fit(ObjectFit::ScaleDown))
    };
    let body = div().size_full().min_h_0().flex();
    match &tab.body {
        Body::Image => body.child(view(tab.path.clone().into())),
        // Drawn by the workspace, which can hand the player a mutable window.
        Body::Video(_) => body,
        Body::Graph(view) => body.child(view.clone()),
        Body::Diff(view) => body.child(view.clone()),
        Body::Text(Text { preview: Some(Preview::Svg(image)), .. }) => {
            body.child(view(image.clone().into()))
        }
        Body::Text(Text { preview: Some(Preview::Markdown(source)), .. }) => body.child(
            div().size_full().flex().justify_center().child(
                crate::markdown::view(source.clone(), cx),
            ),
        ),
        Body::Text(text) => body.child(
            Editor::new(&text.editor)
                .readonly(text.readonly)
                .bordered(false)
                .h_full()
                .flex_1()
                .min_w_0()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.mono_font_size),
        ),
    }
}

pub fn has_ext(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

pub fn is_image(path: &Path) -> bool {
    has_ext(path, &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico"])
}

pub fn language(path: &Path) -> &'static str {
    // Lockfiles and dotfiles say nothing through their extension.
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or_default();
    let by_name = match file {
        "Cargo.lock" | "poetry.lock" | "uv.lock" | "Pipfile" => Some("toml"),
        "flake.lock" | ".babelrc" | ".prettierrc" => Some("json"),
        "Makefile" | "makefile" | "GNUmakefile" => Some("make"),
        "CMakeLists.txt" => Some("cmake"),
        "Gemfile" | "Rakefile" | "Podfile" => Some("ruby"),
        ".bashrc" | ".bash_profile" | ".zshrc" | ".zprofile" | ".profile" | ".env" => Some("bash"),
        _ => None,
    };
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    let name = by_name.unwrap_or(match ext.as_str() {
        "jsx" | "mjs" | "cjs" => "javascript",
        "mts" | "cts" => "typescript",
        "h" => "c",
        "hpp" | "hh" | "cc" | "cxx" => "cpp",
        "htm" | "svg" => "html",
        "zsh" => "bash",
        "mk" => "make",
        "patch" => "diff",
        ext => ext,
    });
    Language::from_str(name).name()
}

/// Driven by bench/measure-open.sh: what opening one more file of an already
/// open language costs the highlighter.
#[std::prelude::rust_2024::test]
#[ignore = "benchmark"]
fn bench_highlighter() {
    use gpui_kit::component::highlighter::SyntaxHighlighter;
    std::hint::black_box(SyntaxHighlighter::new("rust"));
    let start = std::time::Instant::now();
    for _ in 0..20 {
        std::hint::black_box(SyntaxHighlighter::new("rust"));
    }
    let ms = start.elapsed().as_secs_f64() * 1000. / 20.;
    println!("{{\"open_ms\": {ms}}}");
}

#[std::prelude::rust_2024::test]
fn cached_queries_highlight_like_fresh_ones() {
    use gpui_kit::component::{
        highlighter::{HighlightTheme, SyntaxHighlighter},
        input::Rope,
    };
    // Go is used by no other test, so the first highlighter compiles the queries.
    let code = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tx := 42 // answer\n\tfmt.Println(\"hi\", x)\n}\n";
    let rope = Rope::from(code);
    let theme = HighlightTheme::default_dark();
    let styles = || {
        let mut highlighter = SyntaxHighlighter::new("go");
        highlighter.update(None, &rope, None);
        highlighter.styles(&(0..code.len()), &*theme)
    };
    let fresh = styles();
    assert!(!fresh.is_empty());
    assert_eq!(fresh, styles());
}

#[std::prelude::rust_2024::test]
fn languages() {
    assert_eq!(language(Path::new("a/main.rs")), "rust");
    assert_eq!(language(Path::new("x.TSX")), "tsx");
    assert_eq!(language(Path::new("x.hpp")), "cpp");
    assert_eq!(language(Path::new("Cargo.lock")), "toml");
    assert_eq!(language(Path::new("Makefile")), "make");
    assert_eq!(language(Path::new(".zshrc")), "bash");
    assert_eq!(language(Path::new("LICENSE")), "text");
}
