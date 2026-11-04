use std::fmt::Write;

use gpui_kit::base::text::{MarkdownParseContext, markdown_ast::Node};
use unicode_width::UnicodeWidthChar;

pub fn source(node: &Node, cx: &MarkdownParseContext<'_>) -> Option<String> {
    let text = match node {
        Node::Paragraph(_) => cx.node_source(node)?,
        Node::Code(code)
            if matches!(
                code.lang.as_deref().unwrap_or(""),
                "" | "text" | "plain" | "plaintext" | "ascii" | "tree" | "diagram"
            ) =>
        {
            &code.value
        }
        _ => return None,
    };
    is_diagram(text).then(|| text.to_string())
}

fn is_diagram(source: &str) -> bool {
    if source.lines().count() < 2 {
        return false;
    }
    let rows = source
        .lines()
        .filter(|line| {
            line.trim_start()
                .starts_with(|c| ('\u{2500}'..='\u{257f}').contains(&c))
        })
        .count();
    let tree = source.lines().any(|line| {
        ["├─", "└─", "┣━", "┗━", "╠═", "╚═"]
            .iter()
            .any(|prefix| line.trim_start().starts_with(prefix))
    });
    (rows >= 2 && source.chars().any(|c| "│┃┌┐└┘├┤┬┴┼╔╗╚╝╠╣╦╩╬║".contains(c))) || tree
}

// The grid supplies geometry, not graph semantics: labels and branch positions stay where written.
pub fn render(source: &str, color: &str) -> Result<(String, f32, f32), String> {
    if source.len() > 32_768 || source.lines().count() > 512 {
        return Err("Text diagram exceeds 32 KiB or 512 lines".into());
    }
    const CELL: f32 = 10.;
    const ROW: f32 = 22.;
    const PAD: f32 = 12.;
    let mut paths = String::new();
    let mut labels = String::new();
    let mut arrows = String::new();
    let mut max_col = 0;
    let mut rows = 0;
    for (row, line) in source.lines().enumerate() {
        rows += 1;
        let mut col = 0;
        let mut label = String::new();
        let mut label_col = 0;
        let chars: Vec<_> = line.chars().collect();
        for (ix, &ch) in chars.iter().enumerate() {
            let x = PAD + (col as f32 + 0.5) * CELL;
            let y = PAD + (row as f32 + 0.5) * ROW;
            let arrow = match ch {
                '→' | '⟶' | '⟹' | '⇒' => Some((x - CELL / 2., y, x + CELL / 2., y)),
                '←' | '⟵' | '⇐' => Some((x + CELL / 2., y, x - CELL / 2., y)),
                '↑' | '⇑' => Some((x, y + ROW / 2., x, y - ROW / 2.)),
                '↓' | '⇓' => Some((x, y - ROW / 2., x, y + ROW / 2.)),
                '>' if ix > 0 && "─━═-".contains(chars[ix - 1]) => {
                    Some((x - CELL / 2., y, x + CELL / 2., y))
                }
                '<' if chars.get(ix + 1).is_some_and(|ch| "─━═-".contains(*ch)) => {
                    Some((x + CELL / 2., y, x - CELL / 2., y))
                }
                _ => None,
            };
            let sides = sides(ch);
            let dashed = ch == '-'
                && (ix > 0 && "─━═-<".contains(chars[ix - 1])
                    || chars.get(ix + 1).is_some_and(|ch| "─━═->".contains(*ch)));
            let diagonal = "╱╲╳".contains(ch);
            if sides != 0 || arrow.is_some() || dashed || diagonal {
                flush_label(&mut labels, &mut label, label_col, row);
                if let Some((x1, y1, x2, y2)) = arrow {
                    _ = write!(
                        arrows,
                        "<path d=\"M{x1} {y1}L{x2} {y2}\" marker-end=\"url(#arrow)\"/>"
                    );
                } else if diagonal {
                    if ch != '╲' {
                        _ = write!(
                            paths,
                            "M{} {}L{} {}",
                            x - CELL / 2.,
                            y + ROW / 2.,
                            x + CELL / 2.,
                            y - ROW / 2.
                        );
                    }
                    if ch != '╱' {
                        _ = write!(
                            paths,
                            "M{} {}L{} {}",
                            x - CELL / 2.,
                            y - ROW / 2.,
                            x + CELL / 2.,
                            y + ROW / 2.
                        );
                    }
                } else {
                    let sides = if dashed { 3 } else { sides };
                    for (bit, dx, dy) in [
                        (1, -CELL / 2., 0.),
                        (2, CELL / 2., 0.),
                        (4, 0., -ROW / 2.),
                        (8, 0., ROW / 2.),
                    ] {
                        if sides & bit != 0 {
                            _ = write!(paths, "M{x} {y}L{} {}", x + dx, y + dy);
                        }
                    }
                }
            } else {
                if label.is_empty() {
                    label_col = col;
                }
                if ch == '\t' {
                    label.push_str(&" ".repeat(4 - col % 4));
                } else {
                    label.push(ch);
                }
            }
            col += if ch == '\t' {
                4 - col % 4
            } else {
                ch.width().unwrap_or(0)
            };
        }
        flush_label(&mut labels, &mut label, label_col, row);
        max_col = max_col.max(col);
    }
    let width = max_col as f32 * CELL + PAD * 2.;
    let height = rows as f32 * ROW + PAD * 2.;
    let raster_width = width * 2.;
    let raster_height = height * 2.;
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{raster_width}\" height=\"{raster_height}\" viewBox=\"0 0 {width} {height}\"><defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"6\" markerHeight=\"6\" orient=\"auto-start-reverse\"><path d=\"M0 1L9 5L0 9\" fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\"/></marker></defs><g fill=\"none\" stroke=\"{color}\" stroke-width=\"1.5\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"{paths}\"/>{arrows}</g><g fill=\"{color}\" font-family=\"sans-serif\" font-size=\"14\" xml:space=\"preserve\">{labels}</g></svg>"
    );
    Ok((svg, width, height))
}

fn flush_label(svg: &mut String, label: &mut String, col: usize, row: usize) {
    if !label.trim().is_empty() {
        let x = 12. + col as f32 * 10.;
        let y = 12. + (row as f32 + 0.5) * 22. + 5.;
        // Explicit width preserves alignment even when the UI font is proportional or CJK.
        let width = label
            .chars()
            .map(|ch| ch.width().unwrap_or(0))
            .sum::<usize>() as f32
            * 10.;
        _ = write!(
            svg,
            "<text x=\"{x}\" y=\"{y}\" textLength=\"{width}\" lengthAdjust=\"spacingAndGlyphs\">{}</text>",
            escape(label)
        );
    }
    label.clear();
}

fn escape(text: &str) -> String {
    text.replace(
        |ch: char| ch < ' ' || matches!(ch, '\u{fffe}' | '\u{ffff}'),
        "�",
    )
    .replace('&', "&amp;")
    .replace('<', "&lt;")
    .replace('>', "&gt;")
    .replace('"', "&quot;")
    .replace('\'', "&apos;")
}

// Left, right, up, down; line weight is intentionally normalized to one drawing style.
fn sides(ch: char) -> u8 {
    match ch {
        '─' | '━' | '═' | '┄' | '┅' | '┈' | '┉' | '╌' | '╍' => 3,
        '│' | '┃' | '║' | '┆' | '┇' | '┊' | '┋' | '╎' | '╏' => 12,
        '┌' | '┍' | '┎' | '┏' | '╔' | '╒' | '╓' | '╭' => 10,
        '┐' | '┑' | '┒' | '┓' | '╗' | '╕' | '╖' | '╮' => 9,
        '└' | '┕' | '┖' | '┗' | '╚' | '╘' | '╙' | '╰' => 6,
        '┘' | '┙' | '┚' | '┛' | '╝' | '╛' | '╜' | '╯' => 5,
        '├' | '┝' | '┞' | '┟' | '┠' | '┡' | '┢' | '┣' | '╠' | '╞' | '╟' => 14,
        '┤' | '┥' | '┦' | '┧' | '┨' | '┩' | '┪' | '┫' | '╣' | '╡' | '╢' => 13,
        '┬' | '┭' | '┮' | '┯' | '┰' | '┱' | '┲' | '┳' | '╦' | '╤' | '╥' => 11,
        '┴' | '┵' | '┶' | '┷' | '┸' | '┹' | '┺' | '┻' | '╩' | '╧' | '╨' => 7,
        '┼'..='╋' | '╬' | '╪' | '╫' => 15,
        '╴' | '╸' => 1,
        '╵' | '╹' => 4,
        '╶' | '╺' => 2,
        '╷' | '╻' => 8,
        '╼' | '╾' => 3,
        '╽' | '╿' => 12,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn redraws_lines_arrows_and_labels_as_vectors() {
        let source = "┌─────┐    ┌─────┐\n│入力 │───>│出力 │\n└─────┘    └─────┘";
        assert!(super::is_diagram(source));
        let (svg, width, height) = super::render(source, "#ffffff").unwrap();
        assert!(svg.contains("<path"));
        assert!(svg.contains("marker-end"));
        assert!(svg.contains("入力"));
        assert!(svg.contains("出力"));
        assert!(!svg.contains('┌') && !svg.contains('─') && !svg.contains('│'));
        assert!(width > 0. && height > 0.);
    }

    #[test]
    fn recognizes_trees_but_not_ordinary_prose() {
        for source in [
            "root/\n├── src/\n│   └── main.rs\n└── Cargo.toml",
            "root/\n└── README.md",
        ] {
            assert!(super::is_diagram(source));
        }
        for source in [
            "普通の段落\n折り返して表示",
            "A ── B",
            "装飾\n────────",
            "A\n────────\nB\n────────",
            "例の記号│です\n本文の├です",
        ] {
            assert!(!super::is_diagram(source), "{source}");
        }
    }

    #[test]
    fn escapes_labels_and_counts_full_width_characters() {
        let (svg, width, _) = super::render("├── <script>&\n└── 日本語", "#000000").unwrap();
        assert!(svg.contains("&lt;script&gt;&amp;"));
        assert!(!svg.contains("<script>"));
        assert!(width >= 120.);
        let (tabs, _, _) = super::render("root\n\t└── child", "#000000").unwrap();
        assert!(!tabs.contains('\t'));
    }
}
