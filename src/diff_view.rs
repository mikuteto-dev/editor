//! A commit or working-copy change shown as a diff: one card per file, syntax colored, unified
//! or side by side. Only the rows on screen are drawn; large files start folded, and long ones
//! show their first lines with the rest a click away.

use std::{collections::HashMap, path::Path, rc::Rc};

use gpui_kit::{
    component::{
        ActiveTheme, Selectable, Sizable,
        button::{Button, ButtonVariants},
        diff::{Diff, DiffFile, DiffHoverHighlight, DiffMode, DiffState},
        h_flex, v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use crate::{git::CommitInfo, icons, tab::language};

/// Files with more changed lines than this open folded; a click on the header opens them.
const FOLD_LINES: usize = 1_000;
/// Lines of a file's diff shown at first, and added by each "Show more".
const PAGE_LINES: usize = 500;

/// One file's part of the patch, kept whole so more of it can be shown later.
pub struct Part {
    patch: String,
    shown: usize,
    /// Lines from the first hunk on.
    total: usize,
    added: usize,
    removed: usize,
    file: DiffFile,
}

impl Part {
    fn hidden(&self) -> usize {
        self.total.saturating_sub(self.shown)
    }
}

pub struct DiffView {
    title: SharedString,
    meta: Option<SharedString>,
    body: Option<SharedString>,
    parts: Vec<Part>,
    state: Entity<DiffState>,
}

impl DiffView {
    pub fn new(title: SharedString, info: Option<CommitInfo>, parts: Vec<Part>, cx: &mut Context<Self>) -> Self {
        let folded: Vec<SharedString> =
            parts.iter().filter(|p| p.added + p.removed > FOLD_LINES).map(|p| p.file.path().clone()).collect();
        let files: Vec<DiffFile> = parts.iter().map(|p| p.file.clone()).collect();
        let state = cx.new(|cx| {
            let mut state = DiffState::new(files, cx);
            for path in &folded {
                state.set_file_collapsed(path, true, cx);
            }
            state
        });
        let (meta, body) = match info {
            Some(info) => (
                Some(format!("{} · {} · {}", info.author, info.when, info.hash).into()),
                Some(info.body).filter(|b| !b.is_empty()).map(Into::into),
            ),
            None => (None, None),
        };
        Self { title, meta, body, parts, state }
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

    fn show_more(&mut self, path: &SharedString, cx: &mut Context<Self>) {
        let Some(part) = self.parts.iter_mut().find(|p| p.file.path() == path) else { return };
        part.shown = (part.shown + PAGE_LINES * 4).min(part.total);
        let Some(file) = parse_one(&truncate(&part.patch, part.shown)) else { return };
        part.file = file;
        let files: Vec<DiffFile> = self.parts.iter().map(|p| p.file.clone()).collect();
        // Replacing the files resets the scroll position; come back to the file that grew.
        self.state.update(cx, |state, cx| {
            state.set_files(files, cx);
            state.scroll_to_file(path, cx);
        });
        cx.notify();
    }

    fn set_mode(&mut self, mode: DiffMode, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.set_mode(mode, cx));
        cx.notify();
    }
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

impl Render for DiffView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mode = self.state.read(cx).mode();
        let (added, removed) = self.parts.iter().fold((0, 0), |(a, r), p| (a + p.added, r + p.removed));
        let files = self.parts.len();
        let stats = h_flex()
            .gap_2()
            .text_xs()
            .child(div().text_color(theme.muted_foreground).child(format!("{files} {}", if files == 1 { "file" } else { "files" })))
            .child(div().text_color(theme.green).child(format!("+{added}")))
            .child(div().text_color(theme.red).child(format!("−{removed}")));
        let header = v_flex()
            .flex_none()
            .px_4()
            .py_3()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().min_w_0().truncate().text_base().font_weight(FontWeight::SEMIBOLD).child(self.title.clone()))
                    .child(stats)
                    .child(
                        h_flex()
                            .gap_0p5()
                            .child(
                                Button::new("unified")
                                    .label("Unified")
                                    .ghost()
                                    .xsmall()
                                    .selected(mode == DiffMode::Unified)
                                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Unified, cx))),
                            )
                            .child(
                                Button::new("split")
                                    .label("Split")
                                    .ghost()
                                    .xsmall()
                                    .selected(mode == DiffMode::Split)
                                    .on_click(cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Split, cx))),
                            ),
                    ),
            )
            .children(self.meta.clone().map(|meta| div().text_xs().text_color(theme.muted_foreground).child(meta)))
            .children(self.body.clone().map(|body| div().pt_1().text_sm().text_color(theme.foreground).child(body)));
        let stats: Rc<HashMap<SharedString, (usize, usize, usize)>> =
            Rc::new(self.parts.iter().map(|p| (p.file.path().clone(), (p.added, p.removed, p.hidden()))).collect());
        let view = cx.entity().downgrade();
        v_flex().size_full().child(header).child(
            Diff::new(&self.state)
                .line_number(true)
                .syntax_highlight(true)
                .hover_highlight(DiffHoverHighlight::Line)
                .header_prefix(|file, _, _| {
                    let name = file.path().rsplit('/').next().unwrap_or_default().to_string();
                    img(icons::file(&name)).size_4().flex_none()
                })
                // The file's whole size, and the rest of a long file a click away.
                .render_header_suffix(move |file, _, cx| {
                    let theme = cx.theme();
                    let (added, removed, hidden) = stats.get(file.path()).copied().unwrap_or_default();
                    let path = file.path().clone();
                    let view = view.clone();
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .child(div().text_color(theme.green).child(format!("+{added}")))
                        .child(div().text_color(theme.red).child(format!("−{removed}")))
                        .when(hidden > 0, |el| {
                            el.child(
                                Button::new(SharedString::from(format!("more:{path}")))
                                    .label(format!("Show more · {hidden} lines hidden"))
                                    .ghost()
                                    .xsmall()
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |view, cx| view.show_more(&path, cx)).ok();
                                    }),
                            )
                        })
                })
                .flex_1()
                .min_h_0(),
        )
    }
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

    let parts = DiffView::load(&format!("{new_file}{two}"));
    let got: Vec<_> = parts.iter().map(|p| (p.file.path().to_string(), p.total, p.added, p.removed)).collect();
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
    let parts = DiffView::load(&patch);
    println!("{} lines: git {shown:?}, parse {:?}, {} files", patch.lines().count(), start.elapsed() - shown, parts.len());
}
