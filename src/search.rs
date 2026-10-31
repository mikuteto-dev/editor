use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use gpui_kit::{
    component::{
        ActiveTheme, h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    *,
};
use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::UTF8};
use ignore::WalkState;

use crate::icons;

/// Keeps a broad query from flooding the list and memory.
const LIMIT: usize = 2000;

pub struct Hit {
    pub path: PathBuf,
    pub line: u32,
    text: SharedString,
    range: Range<usize>,
}

pub fn search(root: &Path, query: &str) -> Vec<Hit> {
    let Ok(matcher) = RegexMatcherBuilder::new()
        .case_smart(true)
        .fixed_strings(true)
        .build(query)
    else {
        return Vec::new();
    };
    let hits = Mutex::new(Vec::new());
    let found = AtomicUsize::new(0);
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build_parallel()
        .run(|| {
            let (matcher, hits, found) = (&matcher, &hits, &found);
            let mut searcher = SearcherBuilder::new()
                .binary_detection(BinaryDetection::quit(0))
                .build();
            Box::new(move |entry| {
                if found.load(Ordering::Relaxed) >= LIMIT {
                    return WalkState::Quit;
                }
                let Ok(entry) = entry else { return WalkState::Continue };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                let path = entry.path();
                let mut file_hits = Vec::new();
                // Unreadable or non-UTF-8 files just contribute no hits.
                let _ = searcher.search_path(
                    matcher,
                    path,
                    UTF8(|line, text| {
                        let text = text.trim_end();
                        if let Ok(Some(m)) = matcher.find(text.as_bytes()) {
                            let (text, range) = clip(text, m.start()..m.end());
                            file_hits.push(Hit {
                                path: path.to_path_buf(),
                                line: line as u32 - 1,
                                text: text.into(),
                                range,
                            });
                        }
                        Ok(file_hits.len() < LIMIT)
                    }),
                );
                if !file_hits.is_empty() {
                    found.fetch_add(file_hits.len(), Ordering::Relaxed);
                    hits.lock().unwrap().extend(file_hits);
                }
                WalkState::Continue
            })
        });
    let mut hits = hits.into_inner().unwrap();
    // Threads finish in any order; the panel groups hits by file.
    hits.sort_by(|a, b| a.path.cmp(&b.path).then(a.line.cmp(&b.line)));
    hits.truncate(LIMIT);
    hits
}

// Minified files would otherwise produce megabyte-wide rows.
fn clip(line: &str, m: Range<usize>) -> (String, Range<usize>) {
    let indent = line.len() - line.trim_start().len();
    // Little leading context: the sidebar is narrow and the match must stay visible.
    let mut start = m.start.saturating_sub(16).max(indent).min(m.start);
    while !line.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + 240).min(line.len()).max(m.end);
    while !line.is_char_boundary(end) {
        end += 1;
    }
    (line[start..end].to_string(), m.start - start..m.end - start)
}

pub struct OpenAt(pub PathBuf, pub u32);

impl EventEmitter<OpenAt> for SearchPanel {}

pub struct SearchPanel {
    root: PathBuf,
    input: Entity<InputState>,
    hits: Vec<Hit>,
    rows: Vec<Row>,
    task: Task<()>,
    _input: Subscription,
}

enum Row {
    File(usize),
    Hit(usize),
}

impl SearchPanel {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let _input = cx.subscribe(&input, |this, _, e: &InputEvent, cx| {
            if let InputEvent::Change = e {
                this.query(cx);
            }
        });
        Self {
            root,
            input,
            hits: Vec::new(),
            rows: Vec::new(),
            task: Task::ready(()),
            _input,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn query(&mut self, cx: &mut Context<Self>) {
        let query = self.input.read(cx).value().to_string();
        let root = self.root.clone();
        // Replacing the task drops the previous one, so only the last keystroke searches.
        self.task = cx.spawn(async move |this, cx| {
            smol::Timer::after(Duration::from_millis(150)).await;
            let hits = match query.is_empty() {
                true => Vec::new(),
                false => cx.background_spawn(async move { search(&root, &query) }).await,
            };
            this.update(cx, |this, cx| {
                this.rows = hits
                    .iter()
                    .enumerate()
                    .flat_map(|(ix, hit)| {
                        let first = ix == 0 || hits[ix - 1].path != hit.path;
                        first.then_some(Row::File(ix)).into_iter().chain([Row::Hit(ix)])
                    })
                    .collect();
                this.hits = hits;
                cx.notify();
            })
            .ok();
        });
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        let hover = theme.sidebar_accent;
        let mark = HighlightStyle {
            background_color: Some(theme.selection),
            ..Default::default()
        };
        range
            .map(|ix| {
                let row = h_flex().id(ix).h(px(24.)).w_full().px_2().gap_1p5().rounded_sm();
                match self.rows[ix] {
                    Row::File(hit) => {
                        let path = &self.hits[hit].path;
                        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                        let dir = path
                            .parent()
                            .and_then(|d| d.strip_prefix(&self.root).ok())
                            .map(|d| d.display().to_string())
                            .unwrap_or_default();
                        row.child(img(icons::file(&name)).size_4().flex_none())
                            .child(div().flex_none().child(name))
                            .child(div().truncate().text_xs().text_color(theme.muted_foreground).child(dir))
                    }
                    Row::Hit(hit) => {
                        let Hit { path, line, text, range } = &self.hits[hit];
                        let (path, line) = (path.clone(), *line);
                        row.pl(px(30.))
                            .hover(move |el| el.bg(hover))
                            .child(
                                div()
                                    .truncate()
                                    .child(StyledText::new(text.clone()).with_highlights([(range.clone(), mark)])),
                            )
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(OpenAt(path.clone(), line))))
                    }
                }
            })
            .collect()
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .text_sm()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(div().p_2().child(Input::new(&self.input)))
            .child(
                uniform_list(
                    "hits",
                    self.rows.len(),
                    cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                )
                .flex_1()
                .px_1(),
            )
    }
}

/// Driven by bench/measure-search.sh; the sorted hits go to SEARCH_OUT so
/// before and after can be compared exactly.
#[std::prelude::rust_2024::test]
#[ignore = "benchmark"]
fn bench_search() {
    let root = std::env::var("SEARCH_ROOT").expect("SEARCH_ROOT");
    let query = std::env::var("SEARCH_QUERY").expect("SEARCH_QUERY");
    let start = std::time::Instant::now();
    let hits = std::hint::black_box(search(Path::new(&root), &query));
    let ms = start.elapsed().as_secs_f64() * 1000.;
    let mut keys: Vec<_> = hits
        .iter()
        .map(|h| format!("{}:{}:{}", h.path.display(), h.line, h.text))
        .collect();
    keys.sort();
    std::fs::write(std::env::var("SEARCH_OUT").expect("SEARCH_OUT"), keys.join("\n")).unwrap();
    println!("{{\"search_ms\": {ms}, \"hits\": {}}}", hits.len());
}

#[std::prelude::rust_2024::test]
fn clips_around_the_match() {
    assert_eq!(clip("    let x = 1;", 8..9), ("let x = 1;".to_string(), 4..5));
    let long = format!("{}needle{}", "a".repeat(500), "b".repeat(500));
    let (text, range) = clip(&long, 500..506);
    assert_eq!(&text[range.clone()], "needle");
    assert!(text.len() <= 246);
    let wide = "あ".repeat(100) + "x";
    let (text, range) = clip(&wide, 300..301);
    assert_eq!(&text[range], "x");
}

#[std::prelude::rust_2024::test]
fn searches_files_skipping_ignored() {
    let dir = std::env::temp_dir().join(format!("editor-search-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join(".gitignore"), "skip.txt\n").unwrap();
    std::fs::write(dir.join("a.rs"), "fn Needle() {}\nnothing\nneedle again\n").unwrap();
    std::fs::write(dir.join("skip.txt"), "needle\n").unwrap();
    let hits = search(&dir, "needle");
    std::fs::remove_dir_all(&dir).unwrap();
    let lines: Vec<_> = hits.iter().map(|h| (h.path.file_name().unwrap().to_owned(), h.line)).collect();
    assert_eq!(lines, [("a.rs".into(), 0), ("a.rs".into(), 2)]);
}
