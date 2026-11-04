use std::ops::Range;

use markdown::mdast::Node;

// editor patch: masking only delimiters keeps every AST byte/line position valid in the original.
pub(super) fn normalize(source: &str, root: &Node) -> Option<String> {
    if !source.contains("\\(") && !source.contains("\\[") {
        return None;
    }
    let mut protected = Vec::new();
    protect(root, &mut protected);
    protected.sort_by_key(|range| range.start);
    let bytes = source.as_bytes();
    let mut result = bytes.to_vec();
    let mut changed = false;
    let mut ix = 0;
    let mut protected_ix = 0;
    let mut open = None;
    while ix + 1 < bytes.len() {
        while protected_ix < protected.len() && protected[protected_ix].end <= ix {
            protected_ix += 1;
        }
        if let Some(range) = protected
            .get(protected_ix)
            .filter(|range| range.contains(&ix))
        {
            open = None;
            ix = range.end;
            continue;
        }
        if bytes[ix] != b'\\' {
            ix += 1;
            continue;
        }
        match (bytes[ix + 1], open) {
            (b'(' | b'[', None) => open = Some((ix, bytes[ix + 1])),
            (b')', Some((start, b'('))) => {
                result[start..start + 2].copy_from_slice(b"$ ");
                result[ix..ix + 2].copy_from_slice(b" $");
                changed = true;
                open = None;
            }
            (b']', Some((start, b'['))) => {
                result[start..start + 2].copy_from_slice(b"$$");
                result[ix..ix + 2].copy_from_slice(b"$$");
                changed = true;
                open = None;
            }
            _ => {}
        }
        ix += 2;
    }
    changed.then(|| String::from_utf8(result).expect("only ASCII delimiters were replaced"))
}

fn protect(node: &Node, ranges: &mut Vec<Range<usize>>) {
    if matches!(
        node,
        Node::Code(_)
            | Node::InlineCode(_)
            | Node::Html(_)
            | Node::Math(_)
            | Node::InlineMath(_)
            | Node::Definition(_)
            | Node::Image(_)
    ) {
        if let Some(position) = node.position() {
            ranges.push(position.start.offset..position.end.offset);
        }
        return;
    }
    if let Some(children) = node.children() {
        if matches!(node, Node::Link(_) | Node::LinkReference(_))
            && let (Some(position), Some(first), Some(last)) = (
                node.position(),
                children.first().and_then(Node::position),
                children.last().and_then(Node::position),
            )
        {
            ranges.push(position.start.offset..first.start.offset);
            ranges.push(last.end.offset..position.end.offset);
        }
        for child in children {
            protect(child, ranges);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_only_math_and_keeps_positions() {
        let source = "日本語 \\(x_1\\)\n\n\\[\r\n\\frac{1}{2}\r\n\\]\n\n`\\(code\\)`\n\n```tex\n\\(code\\)\n```\n\n\\\\(escaped\\\\)\n\n<!-- \\(comment\\) -->\n";
        let root = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).unwrap();
        let normalized = normalize(source, &root).unwrap();
        assert_eq!(
            normalized,
            source.replacen("\\(x_1\\)", "$ x_1 $", 1).replacen(
                "\\[\r\n\\frac{1}{2}\r\n\\]",
                "$$\r\n\\frac{1}{2}\r\n$$",
                1
            )
        );
        assert_eq!(normalized.len(), source.len());
        let options = markdown::ParseOptions {
            constructs: markdown::Constructs {
                math_flow: true,
                math_text: true,
                ..markdown::Constructs::gfm()
            },
            ..markdown::ParseOptions::gfm()
        };
        let root = markdown::to_mdast(&normalized, &options).unwrap();
        let nodes = root.children().unwrap();
        assert!(matches!(&nodes[1], Node::Math(math) if math.value.contains("\\frac{1}{2}")));
    }

    #[test]
    fn unmatched_delimiters_do_not_cross_code() {
        for source in [
            r"plain \\(not math\\)",
            r"\(not `code` math\)",
            r"\(unfinished",
            r"\[unfinished",
        ] {
            let root = markdown::to_mdast(source, &markdown::ParseOptions::gfm()).unwrap();
            assert_eq!(normalize(source, &root), None, "{source}");
        }
    }
}
