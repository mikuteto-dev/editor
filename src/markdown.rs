// The base text view, not gpui-component's wrapper: only the base one takes colors, blockquote
// and inline code styles.
use gpui_kit::{
    base::text::{TextView, TextViewStyle},
    component::{
        ActiveTheme, Icon, Sizable, Theme,
        button::{Button, ButtonVariants},
        h_flex,
    },
    *,
};

const COPY: &[u8] = include_bytes!("../assets/ui/copy.svg");

pub fn view(source: SharedString, cx: &App) -> TextView {
    TextView::markdown("markdown", source)
        .style(style(cx.theme()))
        // The built-in language label pads the block by a whole empty line, so it rides with the copy button instead.
        .code_block_actions(|block, _, cx| {
            let code = block.code();
            h_flex()
                .gap_1()
                .pl_2()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .children(block.lang().filter(|lang| !lang.is_empty()))
                .child(
                    Button::new("copy")
                        .icon(Icon::default().data(COPY))
                        .ghost()
                        .xsmall()
                        .tooltip("Copy")
                        .on_click(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()))),
                )
        })
        .scrollable(true)
        .selectable(true)
        .w_full()
        .max_w(px(880.))
        .px_10()
        .py_8()
        .text_size(px(15.))
        .line_height(relative(1.65))
}

/// Out of reach of `use gpui_kit::*`, whose `test` would make `#[gpui::test]` expand into itself.
#[cfg(test)]
mod tests {
    use gpui::{Context, IntoElement, Render, Window};

    struct Page;
    impl Render for Page {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            super::view("# Title\n\nText with `code`.\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n".into(), cx)
        }
    }

    #[gpui::test]
    fn draws_a_page(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_kit::init);
        let (_, cx) = cx.add_window_view(|_, _| Page);
        cx.run_until_parked();
        cx.update(|window, cx| {
            _ = window.draw(cx);
        });
    }
}

fn style(theme: &Theme) -> TextViewStyle {
    let border = theme.border;
    let rounded = |radius: f32| StyleRefinement::default().rounded(px(radius));
    TextViewStyle::default()
        .with_foreground(theme.foreground)
        .with_muted_foreground(theme.muted_foreground)
        .with_link(theme.link)
        .with_selection(theme.selection)
        .with_code_background(theme.secondary)
        .with_border(border)
        .with_paragraph_gap(rems(1.))
        .with_heading(move |level| {
            let size = match level {
                1 => 2.,
                2 => 1.5,
                3 => 1.25,
                4 => 1.,
                5 => 0.875,
                _ => 0.85,
            };
            // No margins: a list item's height leaves out its own margin, so the next block would overlap.
            let heading = StyleRefinement::default().text_size(rems(size)).font_weight(FontWeight::SEMIBOLD);
            match level {
                1 | 2 => heading.pb(rems(0.3)).border_b_1().border_color(border),
                _ => heading,
            }
        })
        .with_code_block(rounded(6.).border_1().border_color(border))
        .with_inline_code(HighlightStyle { background_color: Some(theme.secondary), ..Default::default() })
        .with_inline_code_style(rounded(4.))
        .with_blockquote_style(StyleRefinement::default().border_l_4().border_color(border).pl_4().text_color(theme.muted_foreground))
        .with_table(rounded(6.).border_1().border_color(border))
        .with_table_head(StyleRefinement::default().bg(theme.secondary).font_weight(FontWeight::SEMIBOLD))
        .with_table_cell(StyleRefinement::default().px_3().py_1p5())
        .with_dark(theme.is_dark())
}
