/// JSON with comments and trailing commas, as bun.lock and deno.json allow.
pub(super) fn jsonc(text: &str) -> Option<serde_json::Value> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                while chars.next_if(|&n| n != '\n').is_some() {}
            }
            ('/', Some('*')) => {
                chars.next();
                while let Some(n) = chars.next() {
                    if n == '*' && chars.next_if_eq(&'/').is_some() {
                        break;
                    }
                }
            }
            (']' | '}', _) => {
                let trimmed = out.trim_end().len();
                if out[..trimmed].ends_with(',') {
                    out.truncate(trimmed - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    serde_json::from_str(&out).ok()
}

pub(super) fn quoted(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find(['"', '\'']) {
        let q = rest.as_bytes()[start] as char;
        let tail = &rest[start + 1..];
        let Some(end) = tail.find(q) else { break };
        out.push(&tail[..end]);
        rest = &tail[end + 1..];
    }
    out
}

/// Tests read package-manager layouts from a scratch directory.
#[cfg(test)]
pub(super) fn scratch(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("editor-lic-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

#[std::prelude::rust_2024::test]
fn reads_jsonc() {
    let text = "{\n  // comment\n  \"a\": \"x // not a comment\", /* block */\n  \"b\": [1, 2,],\n}\n";
    assert_eq!(jsonc(text), Some(serde_json::json!({"a": "x // not a comment", "b": [1, 2]})));
}

#[std::prelude::rust_2024::test]
fn reads_quoted_strings() {
    assert_eq!(quoted(r#"gem "rails", '~> 7'"#), ["rails", "~> 7"]);
    assert!(quoted("no quotes").is_empty());
}
