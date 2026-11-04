use std::sync::Arc;

use gpui::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, TestAppContext,
    Window, div, px,
};
use gpui_kit::{
    base::text::{
        TextView, TextViewState,
        editor_host::{BlockNode, parse_markdown},
    },
    component::{ActiveTheme, Theme},
};

use super::{Graphic, Pending, Source, extensions, prepare, render};

fn math(tex: &str, display: bool) -> Source {
    Source::Math {
        tex: tex.into(),
        display,
    }
}

fn check_svg(graphic: &Graphic) {
    static FONTS: std::sync::LazyLock<Arc<resvg::usvg::fontdb::Database>> =
        std::sync::LazyLock::new(|| {
            let mut fonts = resvg::usvg::fontdb::Database::new();
            fonts.load_system_fonts();
            Arc::new(fonts)
        });
    let options = resvg::usvg::Options {
        fontdb: FONTS.clone(),
        ..Default::default()
    };
    let tree = resvg::usvg::Tree::from_data(graphic.image.bytes(), &options).unwrap();
    let scale = (512. / tree.size().width().max(tree.size().height())).min(1.);
    let mut pixels = resvg::tiny_skia::Pixmap::new(
        (tree.size().width() * scale).ceil() as u32,
        (tree.size().height() * scale).ceil() as u32,
    )
    .unwrap();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixels.as_mut(),
    );
    assert!(
        pixels.pixels().iter().any(|pixel| pixel.alpha() > 0),
        "empty SVG: {}",
        std::str::from_utf8(graphic.image.bytes()).unwrap()
    );
    assert!(pixels.pixels().windows(2).any(|pair| pair[0] != pair[1]));
    assert!(graphic.width > 0. && graphic.height > 0.);
    assert!(graphic.baseline <= graphic.height);
}

#[test]
fn embeds_math_glyphs_and_supports_tex_extensions() {
    for tex in [
        r"x^2 + y_1 = \alpha",
        r"\frac{-b \pm \sqrt{b^2-4ac}}{2a}",
        r"\int_0^\infty e^{-x}\,dx",
        r"\sum_{n=1}^{\infty}\frac{1}{n^2}",
        r"\left(\frac{a}{b}\right)",
        r"\begin{pmatrix}1&2\\3&4\end{pmatrix}",
        r"f(x)=\begin{cases}x&x>0\\-x&x\le0\end{cases}",
        r"\begin{aligned}a&=b+c\\d&=e\end{aligned}",
        r"\begin{align}a&=b\tag{A}\\c&=d\notag\end{align}",
        r"\mathbb{R}\subset\mathbb{C}",
        r"\newcommand{\sq}[1]{#1^2}\sq{x}",
        r"\ce{H2SO4 + 2NaOH -> Na2SO4 + 2H2O}",
        r"\pu{1.5e-3 mol//L}",
        r"\slashed{p}",
        r"\text{速度}\quad v=\frac{d}{t}",
        r"\begin{equation}E=mc^2\end{equation}",
    ] {
        for dark in [false, true] {
            let graphic = render(
                &math(tex, true),
                if dark {
                    gpui::white().to_rgb()
                } else {
                    gpui::black().to_rgb()
                },
                dark,
            )
            .unwrap_or_else(|e| panic!("{tex}: {e}"));
            let svg = std::str::from_utf8(graphic.image.bytes()).unwrap();
            assert!(svg.contains("<path"), "{tex}");
            assert!(
                !svg.contains("<text"),
                "glyph outlines must be embedded: {tex}"
            );
            check_svg(&graphic);
        }
    }
    let inline = render(
        &math(r"\sum_{i=0}^{n}i", false),
        gpui::black().to_rgb(),
        false,
    )
    .unwrap();
    let display = render(
        &math(r"\sum_{i=0}^{n}i", true),
        gpui::black().to_rgb(),
        false,
    )
    .unwrap();
    assert!(display.height > inline.height);
}

#[test]
fn mermaid_init_changes_the_rendered_color_and_spacing() {
    let base = render(&Source::Mermaid("flowchart LR\nA-->B".into()), gpui::black().to_rgb(), false).unwrap();
    let custom = render(&Source::Mermaid("%%{init: {'theme':'base','themeVariables':{'primaryColor':'#ff0000'},'flowchart':{'nodeSpacing':120,'rankSpacing':200}}}%%\nflowchart LR\nA-->B".into()), gpui::black().to_rgb(), false).unwrap();
    assert!(custom.width > base.width);
    assert!(std::str::from_utf8(custom.image.bytes()).unwrap().contains("#ff0000"));
    check_svg(&custom);
}

#[test]
fn renders_major_mermaid_dialects() {
    for source in [
        "flowchart LR\nA[開始] --> B{確認}\nB -->|はい| C[完了]",
        "graph TD\nsubgraph Group\nA --> B\nend",
        "sequenceDiagram\nAlice->>Bob: Hello\nBob-->>Alice: Reply",
        "classDiagram\nAnimal <|-- Cat\nAnimal : +run()",
        "stateDiagram-v2\n[*] --> Idle\nIdle --> Active\nActive --> [*]",
        "erDiagram\nUSER ||--o{ ORDER : places",
        "pie title Share\n\"A\" : 40\n\"B\" : 60",
        "gantt\ndateFormat YYYY-MM-DD\nsection Work\nBuild :a, 2026-01-01, 2d",
        "gitGraph\ncommit\nbranch develop\ncheckout develop\ncommit",
        "mindmap\n  root((Project))\n    Source\n    Tests",
        "timeline\n  title Releases\n  2025 : First\n  2026 : Second",
        "journey\n  title Workflow\n  section Build\n    Test: 5: Developer",
        "xychart-beta\n  x-axis [Jan, Feb, Mar]\n  y-axis \"Count\" 0 --> 10\n  bar [3, 7, 9]",
        "sankey-beta\n\nSource,Target,10",
    ] {
        for dark in [false, true] {
            let graphic = render(
                &Source::Mermaid(source.into()),
                gpui::black().to_rgb(),
                dark,
            )
            .unwrap_or_else(|e| panic!("{source}: {e}"));
            check_svg(&graphic);
        }
    }
}

fn custom_nodes(blocks: &[BlockNode]) -> Vec<&gpui_kit::base::text::MarkdownNode> {
    let mut nodes = Vec::new();
    for block in blocks {
        match block {
            BlockNode::Custom { node, .. } => nodes.push(node),
            BlockNode::Paragraph(paragraph)
            | BlockNode::Heading {
                children: paragraph,
                ..
            } => {
                nodes.extend(
                    paragraph
                        .children()
                        .iter()
                        .filter_map(|inline| inline.as_custom()),
                );
            }
            BlockNode::Root { children, .. }
            | BlockNode::Blockquote { children, .. }
            | BlockNode::List { children, .. }
            | BlockNode::ListItem { children, .. } => nodes.extend(custom_nodes(children)),
            _ => {}
        }
    }
    nodes
}

#[test]
fn recognizes_math_fences_delimiters_and_keeps_source() {
    let source = "日本語 $x^2$ と \\(y_1\\)。\n\n$$\n\\frac{1}{2}\n$$\n\n\\[\r\n\\sqrt{2}\r\n\\]\n\n```tex\n\\alpha\n```\n\n```latex\n\\beta\n```\n\n```math\n\\gamma\n```\n\n```mermaid\ngraph LR; A-->B\n```\n\n- 項目 $z$\n\n> 引用 \\(a+b\\)\n";
    let mut theme = Theme::default();
    theme.foreground = gpui::black();
    let blocks = parse_markdown(source, Arc::new(extensions(&theme))).unwrap();
    let nodes = custom_nodes(&blocks);
    assert_eq!(nodes.len(), 10);
    for node in nodes {
        let range = node.source_range().unwrap();
        assert_eq!(node.as_markdown(), &source[range]);
        let pending = node.data::<Arc<Pending>>().unwrap();
        assert!(!pending.started.load(std::sync::atomic::Ordering::Acquire));
        check_svg(&render(&pending.source, pending.color, pending.dark).unwrap());
    }
}

#[test]
fn preview_fixture_graphics_are_renderable() {
    let source = include_str!("../../../tests/fixtures/markdown.md");
    let mut theme = Theme::default();
    theme.foreground = gpui::black();
    let blocks = parse_markdown(source, Arc::new(extensions(&theme))).unwrap();
    let nodes = custom_nodes(&blocks);
    assert!(nodes.len() >= 12);
    for node in nodes {
        let pending = node.data::<Arc<Pending>>().unwrap();
        let graphic = render(&pending.source, pending.color, pending.dark)
            .unwrap_or_else(|error| panic!("{}: {error}", node.as_markdown()));
        check_svg(&graphic);
    }
}

#[test]
fn converts_fenced_and_unfenced_box_drawings_to_graphics() {
    let diagram = "┌─────┐    ┌─────┐\n│入力 │───>│出力 │\n└─────┘    └─────┘";
    for source in [
        diagram.to_string(),
        format!("```text\n{diagram}\n```"),
        format!("```plaintext\n{diagram}\n```"),
        format!("```\n{diagram}\n```"),
        "root/\n├── src/\n└── README.md".to_string(),
    ] {
        let blocks = parse_markdown(&source, Arc::new(extensions(&Theme::default()))).unwrap();
        let nodes = custom_nodes(&blocks);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].as_markdown(), source);
        let pending = nodes[0].data::<Arc<Pending>>().unwrap();
        assert!(matches!(&pending.source, Source::Art(_)));
        let graphic = render(&pending.source, gpui::black().to_rgb(), false).unwrap();
        check_svg(&graphic);
        let svg = std::str::from_utf8(graphic.image.bytes()).unwrap();
        assert!(!svg.contains('├') && !svg.contains('┌'));
    }
    let blocks = parse_markdown(
        &format!("```rust\n{diagram}\n```"),
        Arc::new(extensions(&Theme::default())),
    )
    .unwrap();
    assert!(custom_nodes(&blocks).is_empty());
}

#[test]
fn standalone_display_delimiters_are_blocks() {
    for source in [r"$$E=mc^2$$", r"\[E=mc^2\]"] {
        let blocks = parse_markdown(source, Arc::new(extensions(&Theme::default()))).unwrap();
        assert!(
            matches!(blocks.as_slice(), [BlockNode::Custom { .. }]),
            "{blocks:?}"
        );
    }
}

#[test]
fn recognizes_github_inline_math_and_display_dollars() {
    let source = r"$`x^2`$ and $$\sum_{i=0}^{n}i$$";
    let blocks = parse_markdown(source, Arc::new(extensions(&Theme::default()))).unwrap();
    let nodes = custom_nodes(&blocks);
    assert_eq!(nodes.len(), 2);
    let inline = nodes[0].data::<Arc<Pending>>().unwrap();
    assert!(matches!(&inline.source, Source::Math { tex, display: false } if tex == "x^2"));
    let display = nodes[1].data::<Arc<Pending>>().unwrap();
    assert!(matches!(
        &display.source,
        Source::Math { tex, display: true } if tex == r"\sum_{i=0}^{n}i"
    ));
}

#[test]
fn leaves_prose_code_and_escapes_alone() {
    let source = r"Prices $5 and $10. Escaped \$x\$. `\(code\)` and `$x$`. \\(escaped\\)";
    let blocks = parse_markdown(source, Arc::new(extensions(&Theme::default()))).unwrap();
    assert!(custom_nodes(&blocks).is_empty());
    let blocks = parse_markdown(
        "```rust\nlet x = \"$x$\";\n```",
        Arc::new(extensions(&Theme::default())),
    )
    .unwrap();
    assert!(matches!(blocks.as_slice(), [BlockNode::CodeBlock(_)]));
}

#[test]
fn rejects_invalid_or_excessive_input() {
    for source in [
        math(r"\undefinedCommand{x}", true),
        math(r"\frac{1}", true),
        math(r"\def\a{\a}\a", true),
        math(&"x".repeat(16_385), true),
        Source::Mermaid("not a diagram".into()),
        Source::Art("x".repeat(32_769)),
        Source::Art(format!("root\n└── {}", "x".repeat(30_000))),
        Source::Mermaid("flowchart LR\n".repeat(513)),
        Source::Mermaid(format!(
            "flowchart LR\n{}",
            (0..257)
                .map(|i| format!("N{i}[Node]\n"))
                .collect::<String>()
        )),
    ] {
        assert!(render(&source, gpui::black().to_rgb(), false).is_err());
    }
}

#[gpui::test]
fn asynchronous_render_is_retained_and_errors_keep_source(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    for source in [
        "$$\\frac{1}{2}$$",
        "```mermaid\nflowchart LR; A-->B\n```",
        "$$\\undefinedCommand{x}$$",
    ] {
        let theme = cx.update(|cx| cx.theme().clone());
        let blocks = parse_markdown(source, Arc::new(extensions(&theme))).unwrap();
        let node = custom_nodes(&blocks)[0].clone();
        cx.update(|cx| {
            assert!(prepare(&node, cx).is_none());
            assert!(prepare(&node, cx).is_none());
        });
        cx.run_until_parked();
        let result = cx.update(|cx| {
            prepare(&node, cx)
                .unwrap()
                .as_ref()
                .map(|g| g.image.clone())
                .map_err(Clone::clone)
        });
        assert_eq!(result.is_err(), source.contains("undefinedCommand"));
        assert_eq!(node.as_markdown(), source);
        if let Ok(image) = result {
            cx.update(|cx| {
                let first = prepare(&node, cx).unwrap().as_ref().unwrap();
                assert!(Arc::ptr_eq(&image, &first.image));
                assert!(image.to_image_data(cx.svg_renderer()).is_ok());
            });
        }
    }
}

struct Page {
    state: Entity<TextViewState>,
    width: f32,
}
impl Render for Page {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(self.width)).h(px(600.)).child(
            TextView::new(&self.state)
                .selection_format(gpui_kit::base::text::SelectionFormat::Source)
                .markdown_extensions(extensions(cx.theme()))
                .style(super::super::style(cx.theme()))
                .scrollable(true),
        )
    }
}

#[gpui::test]
fn graphics_reflow_without_overlapping_following_blocks(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = "Before $\\frac{1}{2}$ after.\n\n$$\\begin{pmatrix}1&2\\\\3&4\\end{pmatrix}$$\n\nFollowing paragraph.\n\n```mermaid\nflowchart LR; A[Start]-->B[Finish]\n```\n\nroot/\n├── src/\n└── README.md\n\nEnd.";
    let (page, cx) = cx.add_window_view(|_, cx| Page {
        state: cx.new(|cx| TextViewState::markdown(source, cx)),
        width: 720.,
    });
    for width in [720., 260.] {
        page.update(cx, |page, cx| {
            page.width = width;
            cx.notify();
        });
        for _ in 0..4 {
            cx.update(|window, cx| {
                _ = window.draw(cx);
            });
            cx.run_until_parked();
        }
        page.read_with(cx, |page, cx| {
            let state = page.state.read(cx);
            assert_eq!(state.rendered_text().source(), source);
            let list = state.list_state();
            assert_eq!(list.item_count(), 6);
            let bounds: Vec<_> = (0..6).filter_map(|ix| list.bounds_for_item(ix)).collect();
            assert_eq!(bounds.len(), 6);
            assert!(bounds.iter().all(|b| b.size.height > px(0.)));
            for pair in bounds.windows(2) {
                assert!(pair[0].bottom() <= pair[1].top());
            }
        });
    }
}

#[gpui::test]
fn completed_resources_remeasure_offscreen_rows_and_keep_copy(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let source = (0..24)
        .map(|i| format!("Row {i}: $\\frac{{1}}{{\\frac{{1}}{{x}}}}$"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let (page, cx) = cx.add_window_view(|_, cx| Page {
        state: cx.new(|cx| TextViewState::markdown("", cx)),
        width: 720.,
    });
    page.update(cx, |page, cx| {
        page.state
            .update(cx, |state, cx| state.set_text(&source, cx));
        cx.notify();
    });
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
    let height = |page: &Entity<Page>, cx: &gpui::VisualTestContext| {
        page.read_with(cx, |page, cx| {
            let list = page.state.read(cx).list_state();
            list.max_offset_for_scrollbar().y + list.viewport_bounds().size.height
        })
    };
    let before = height(&page, cx);
    for _ in 0..5 {
        cx.run_until_parked();
        cx.update(|window, cx| {
            _ = window.draw(cx);
        });
    }
    assert!(
        height(&page, cx) > before + px(100.),
        "before={before:?}, after={:?}",
        height(&page, cx)
    );
    page.update(cx, |page, cx| {
        page.state.update(cx, |state, cx| {
            state.select_all(cx);
            assert_eq!(state.selected_text(), source);
            state.list_state().scroll_to_end();
        });
        cx.notify();
    });
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
    page.read_with(cx, |page, cx| {
        let list = page.state.read(cx).list_state();
        assert_eq!(list.item_count(), 24);
        let end = list.bounds_for_item(23).unwrap();
        assert!(end.bottom() <= list.viewport_bounds().bottom() + px(1.));
    });
}
