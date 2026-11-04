use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use gpui_kit::{
    base::text::{
        InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode, MarkdownParseContext,
        MarkdownPlugin, markdown_ast::Node,
    },
    component::{ActiveTheme, Theme},
    *,
};
use ratex_types::{color::Color, math_style::MathStyle};

#[cfg(test)]
mod tests;

pub fn extensions(theme: &Theme) -> MarkdownExtensions {
    let color = theme.foreground.to_rgb();
    let dark = theme.is_dark();
    MarkdownExtensions::default()
        .tex_math()
        .plugin(Graphics {
            block: false,
            color,
            dark,
        })
        .plugin(Graphics {
            block: true,
            color,
            dark,
        })
        .parser_revision((u64::from(u32::from(color)) << 1) | u64::from(dark))
}

struct Graphics {
    block: bool,
    color: Rgba,
    dark: bool,
}

#[derive(Clone)]
enum Source {
    Math { tex: String, display: bool },
    Mermaid(String),
    Art(String),
}

struct Pending {
    source: Source,
    color: Rgba,
    dark: bool,
    started: AtomicBool,
    result: OnceLock<Result<Graphic, String>>,
}

struct Graphic {
    image: Arc<Image>,
    width: f32,
    height: f32,
    baseline: f32,
}

impl MarkdownPlugin for Graphics {
    fn name(&self) -> &str {
        if self.block { "diagram" } else { "formula" }
    }

    fn is_block(&self) -> bool {
        self.block
    }

    fn parse(&self, node: &Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let source = if self.block
            && let Some(source) = super::art::source(node, cx)
        {
            Source::Art(source)
        } else {
            match node {
                Node::InlineMath(math) if !self.block => {
                    let literal = cx.node_source(node)?;
                    // Single-dollar math follows Pandoc's spacing rule, so prices stay prose.
                    if literal.starts_with('$')
                        && !literal.starts_with("$$")
                        && (literal.trim_matches('$').starts_with(char::is_whitespace)
                            || literal.trim_matches('$').ends_with(char::is_whitespace)
                            || cx
                                .source()
                                .get(node.position()?.end.offset..)?
                                .starts_with(|c: char| c.is_ascii_digit()))
                    {
                        return None;
                    }
                    let tex = if literal.starts_with("$`") && literal.ends_with("`$") {
                        math.value.strip_prefix('`')?.strip_suffix('`')?.to_string()
                    } else {
                        math.value.clone()
                    };
                    Source::Math {
                        tex,
                        display: literal.starts_with("$$") || literal.starts_with(r"\["),
                    }
                }
                Node::Paragraph(paragraph) if self.block => {
                    let [Node::InlineMath(math)] = paragraph.children.as_slice() else {
                        return None;
                    };
                    let literal = cx.node_source(node)?.trim();
                    if !literal.starts_with("$$") && !literal.starts_with(r"\[") {
                        return None;
                    }
                    Source::Math {
                        tex: math.value.clone(),
                        display: true,
                    }
                }
                Node::Math(math) if self.block => Source::Math {
                    tex: math.value.clone(),
                    display: true,
                },
                Node::Code(code) if self.block => {
                    match code.lang.as_deref()?.to_ascii_lowercase().as_str() {
                        "mermaid" => Source::Mermaid(code.value.clone()),
                        "math" | "tex" | "latex" => Source::Math {
                            tex: code.value.clone(),
                            display: true,
                        },
                        _ => return None,
                    }
                }
                _ => return None,
            }
        };
        let literal = cx.node_source(node).unwrap_or_default();
        Some(
            MarkdownNode::new(
                self.name(),
                Arc::new(Pending {
                    source,
                    color: self.color,
                    dark: self.dark,
                    started: AtomicBool::new(false),
                    result: OnceLock::new(),
                }),
            )
            .text(literal.to_string())
            .markdown(literal.to_string()),
        )
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let mut block = div().w_full().min_w_0().py_2();
        match prepare(node, cx) {
            Some(Ok(graphic)) => {
                let scale = if matches!(
                    node.data::<Arc<Pending>>().unwrap().source,
                    Source::Math { .. }
                ) {
                    f32::from(window.text_style().font_size.to_pixels(window.rem_size()))
                } else {
                    1.
                };
                block = block.flex().justify_center().child(
                    img(graphic.image.clone())
                        .w_full()
                        .max_w(px(graphic.width * scale))
                        .aspect_ratio(graphic.width / graphic.height),
                );
            }
            result => {
                block = block.flex().flex_col().gap_2();
                if let Some(Err(error)) = result {
                    block = block.child(div().text_color(cx.theme().red).child(error.clone()));
                }
                block = block.child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(node.as_text().to_string()),
                );
            }
        }
        block
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let graphic = prepare(node, cx)?.as_ref().ok()?;
        let font = f32::from(context.font_size());
        Some(
            InlineElement::new(
                img(graphic.image.clone())
                    .w(px(graphic.width * font))
                    .h(px(graphic.height * font)),
            )
            .with_baseline(px(graphic.baseline * font)),
        )
    }
}

fn prepare<'a>(node: &'a MarkdownNode, cx: &mut App) -> Option<&'a Result<Graphic, String>> {
    let pending = node.data::<Arc<Pending>>()?;
    if !pending.started.swap(true, Ordering::AcqRel) {
        let pending = pending.clone();
        let owner = gpui_kit::base::GlobalState::global(cx)
            .text_view_state()
            .map(|state| state.downgrade());
        let task = cx.background_executor().spawn(async move {
            let result =
                std::panic::catch_unwind(|| render(&pending.source, pending.color, pending.dark))
                    .unwrap_or_else(|_| Err("Renderer failed".into()));
            _ = pending.result.set(result);
        });
        cx.spawn(async move |cx| {
            task.await;
            if let Some(owner) = owner {
                _ = owner.update(cx, |state, cx| state.invalidate_inline_layout(cx));
            } else {
                cx.refresh();
            }
        })
        .detach();
    }
    pending.result.get()
}

fn render(source: &Source, color: Rgba, dark: bool) -> Result<Graphic, String> {
    let (svg, width, height, baseline) = match source {
        Source::Math { tex, display } => {
            if tex.len() > 16_384 {
                return Err("Formula exceeds 16 KiB".into());
            }
            let ast = ratex_parser::parser::parse(tex).map_err(|e| e.to_string())?;
            let options = ratex_layout::LayoutOptions::default()
                .with_style(if *display {
                    MathStyle::Display
                } else {
                    MathStyle::Text
                })
                .with_color(Color::new(color.r, color.g, color.b, color.a));
            let layout = ratex_layout::layout(&ast, &options);
            ratex_layout::validate_layout(&layout)?;
            let list = ratex_layout::to_display_list(&layout);
            let svg = ratex_svg::render_to_svg_with_color_syntax(
                &list,
                &ratex_svg::SvgOptions {
                    font_size: 32.,
                    padding: 2.,
                    embed_glyphs: true,
                    ..Default::default()
                },
                ratex_svg::SvgColorSyntax::Rgb,
            );
            (
                svg,
                (list.width + 0.125) as f32,
                (list.height + list.depth + 0.125) as f32,
                (list.height + 0.0625) as f32,
            )
        }
        Source::Art(code) => {
            let hex = format!("#{:06x}", u32::from(color) >> 8);
            let (svg, width, height) = super::art::render(code, &hex)?;
            (svg, width, height, height)
        }
        Source::Mermaid(code) => {
            if code.len() > 32_768 || code.lines().count() > 512 {
                return Err("Diagram exceeds 32 KiB or 512 lines".into());
            }
            let parsed =
                mermaid_rs_renderer::parse_mermaid_strict(code).map_err(|e| e.to_string())?;
            if parsed.graph.nodes.len() > 256 || parsed.graph.edges.len() > 512 {
                return Err("Diagram exceeds 256 nodes or 512 edges".into());
            }
            let (theme, config) = super::mermaid::settings(parsed.init_config.as_ref(), dark)?;
            let layout = mermaid_rs_renderer::compute_layout(&parsed.graph, &theme, &config);
            let size = mermaid_rs_renderer::measure_svg_dimensions(&layout, &config, None);
            let svg = mermaid_rs_renderer::render::render_svg_with_dimensions(
                &layout,
                &theme,
                &config,
                Some((size.width * 2., size.height * 2.)),
            );
            (svg, size.width, size.height, size.height)
        }
    };
    let raster_scale: f32 = if matches!(source, Source::Math { .. }) {
        32. * 4. / 3.
    } else {
        2.
    };
    if width * height * raster_scale.powi(2) > 8_000_000. {
        return Err("Rendered graphic exceeds 8 megapixels".into());
    }
    if ![width, height, baseline]
        .iter()
        .all(|n| n.is_finite() && *n > 0.)
        || width > 32_768.
        || height > 32_768.
        || svg.len() > 8 * 1024 * 1024
    {
        return Err("Rendered graphic exceeds size limits".into());
    }
    Ok(Graphic {
        image: Arc::new(Image::from_bytes(ImageFormat::Svg, svg.into_bytes())),
        width,
        height,
        baseline,
    })
}
