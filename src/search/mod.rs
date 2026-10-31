mod panel;

use std::{
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use gpui_kit::SharedString;
use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{BinaryDetection, SearcherBuilder, sinks::UTF8};
use ignore::WalkState;

pub use panel::{OpenAt, SearchPanel};

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
