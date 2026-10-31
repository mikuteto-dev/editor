use std::path::Path;

use gpui_kit::{SharedString, component::diff::DiffFile};

use crate::tab::language;

const PAGE_LINES: usize = 500;

/// One file's part of the patch, kept whole so more of it can be shown later.
pub struct Part {
    patch: String,
    shown: usize,
    total: usize,
    pub added: usize,
    pub removed: usize,
    pub file: DiffFile,
}

impl Part {
    pub fn path(&self) -> &SharedString {
        self.file.path()
    }

    pub fn hidden(&self) -> usize {
        self.total.saturating_sub(self.shown)
    }

    /// Shows the next pages of this file; false when there was nothing more or it failed to parse.
    pub fn show_more(&mut self) -> bool {
        let shown = (self.shown + PAGE_LINES * 4).min(self.total);
        let Some(file) = parse_one(&truncate(&self.patch, shown)) else { return false };
        (self.shown, self.file) = (shown, file);
        true
    }
}

/// Splits and parses off the main thread; a vendored library can make a patch of 100k+ lines.
pub fn load(patch: &str) -> Vec<Part> {
    let mut chunks: Vec<String> = Vec::new();
    for line in patch.lines() {
        if line.starts_with("diff --git ") || chunks.is_empty() {
            chunks.push(String::new());
        }
        let chunk = chunks.last_mut().unwrap();
        chunk.push_str(line);
        chunk.push('\n');
    }
    chunks
        .into_iter()
        .filter_map(|patch| {
            let body = patch.lines().skip_while(|l| !l.starts_with("@@"));
            let (total, added, removed) = body.fold((0, 0, 0), |(t, a, r), l| match l.as_bytes().first() {
                Some(b'+') => (t + 1, a + 1, r),
                Some(b'-') => (t + 1, a, r + 1),
                _ => (t + 1, a, r),
            });
            let file = parse_one(&truncate(&patch, PAGE_LINES))?;
            Some(Part { patch, shown: PAGE_LINES.min(total), total, added, removed, file })
        })
        .collect()
}

fn parse_one(patch: &str) -> Option<DiffFile> {
    let file = DiffFile::parse(patch).ok()?.into_iter().next()?;
    let language = language(Path::new(file.path().as_ref()));
    Some(file.with_language(language))
}

/// The file's patch with at most `limit` lines from its first hunk on. Whole hunks are kept until
/// the budget runs out; the hunk it runs out in is cut and its header counts what remains.
fn truncate(patch: &str, limit: usize) -> String {
    let lines: Vec<&str> = patch.lines().collect();
    let Some(first) = lines.iter().position(|l| l.starts_with("@@")) else { return patch.to_string() };
    if lines.len() - first <= limit {
        return patch.to_string();
    }
    let mut out: Vec<String> = lines[..first].iter().map(|l| l.to_string()).collect();
    let (mut header, mut old, mut new) = (0, 0, 0);
    for line in &lines[first..first + limit] {
        match line.as_bytes().first() {
            Some(b'@') => {
                (header, old, new) = (out.len(), 0, 0);
            }
            Some(b' ') => (old, new) = (old + 1, new + 1),
            Some(b'-') => old += 1,
            Some(b'+') => new += 1,
            _ => {}
        }
        out.push(line.to_string());
    }
    out[header] = recount(&out[header], old, new);
    out.join("\n") + "\n"
}

/// "@@ -a,b +c,d @@ context" with b and d replaced.
fn recount(header: &str, old: usize, new: usize) -> String {
    let mut parts = header.splitn(4, ' ');
    let (Some("@@"), Some(old_range), Some(new_range)) = (parts.next(), parts.next(), parts.next()) else {
        return header.to_string();
    };
    let start = |range: &str| range[1..].split(',').next().unwrap_or("0").to_string();
    let rest = parts.next().unwrap_or("@@");
    format!("@@ -{},{old} +{},{new} {rest}", start(old_range), start(new_range))
}

#[std::prelude::rust_2024::test]
fn cuts_long_files_and_keeps_hunks_valid() {
    let new_file = format!("diff --git a/big.rs b/big.rs\nnew file mode 100644\n--- /dev/null\n+++ b/big.rs\n@@ -0,0 +1,10 @@\n{}", "+x\n".repeat(10));
    let cut = truncate(&new_file, 4);
    assert!(cut.contains("@@ -0,0 +1,3 @@"), "{cut}");
    assert_eq!(cut.lines().filter(|l| *l == "+x").count(), 3);
    let file = parse_one(&cut).unwrap();
    assert_eq!((file.additions(), file.language().to_string()), (3, "rust".to_string()));
    // A file within the budget comes back untouched.
    assert_eq!(truncate(&new_file, 100), new_file);
    // Two hunks: the first stays whole, the second is cut mid-way.
    let two = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@ fn\n-a\n+b\n@@ -10,2 +10,2 @@\n c\n-d\n+e\n";
    let cut = truncate(two, 5);
    assert!(cut.contains("@@ -1 +1 @@ fn") && cut.contains("@@ -10,1 +10,1 @@"), "{cut}");
    assert!(parse_one(&cut).is_some());

    let parts = load(&format!("{new_file}{two}"));
    let got: Vec<_> = parts.iter().map(|p| (p.path().to_string(), p.total, p.added, p.removed)).collect();
    assert_eq!(got, [("big.rs".to_string(), 11, 10, 0), ("a".to_string(), 7, 2, 2)]);
}

/// `cargo test --release parse_root_commit -- --ignored --nocapture` times this repository's first commit.
#[std::prelude::rust_2024::test]
#[ignore = "measures"]
fn parse_root_commit() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let start = std::time::Instant::now();
    let patch = crate::git::show(dir, "994bee0", None);
    let shown = start.elapsed();
    let parts = load(&patch);
    println!("{} lines: git {shown:?}, parse {:?}, {} files", patch.lines().count(), start.elapsed() - shown, parts.len());
}
