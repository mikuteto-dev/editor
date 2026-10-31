use std::{collections::HashMap, path::Path};

use super::git;

/// "author, when · summary" per line; lines not committed yet are None.
pub fn blame(dir: &Path, file: &Path) -> Vec<Option<String>> {
    let file = file.to_string_lossy();
    let Some(out) = git(dir, &["blame", "--porcelain", "--", &file], &[0]) else { return Vec::new() };
    parse(&out, now())
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn parse(out: &str, now: u64) -> Vec<Option<String>> {
    // Porcelain output gives a commit's details only the first time it appears.
    let mut commits: HashMap<&str, (String, u64, String)> = HashMap::new();
    let mut lines = Vec::new();
    let mut current = "";
    for line in out.lines() {
        if line.starts_with('\t') {
            let info = commits.get(current).filter(|_| !current.starts_with("0000000"));
            lines.push(info.map(|(author, time, summary)| format!("{author}, {} · {summary}", ago(now.saturating_sub(*time)))));
            continue;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        if key.len() == 40 && key.chars().all(|c| c.is_ascii_hexdigit()) {
            current = key;
            commits.entry(key).or_default();
        } else if let Some(entry) = commits.get_mut(current) {
            match key {
                "author" => entry.0 = value.to_string(),
                "author-time" => entry.1 = value.parse().unwrap_or_default(),
                "summary" => entry.2 = value.to_string(),
                _ => {}
            }
        }
    }
    lines
}

fn ago(secs: u64) -> String {
    let (n, unit) = match secs {
        s if s < 3600 => (s / 60, "minute"),
        s if s < 86_400 => (s / 3600, "hour"),
        s if s < 30 * 86_400 => (s / 86_400, "day"),
        s if s < 365 * 86_400 => (s / (30 * 86_400), "month"),
        s => (s / (365 * 86_400), "year"),
    };
    match n {
        0 => "just now".to_string(),
        1 => format!("1 {unit} ago"),
        n => format!("{n} {unit}s ago"),
    }
}

#[std::prelude::rust_2024::test]
fn reads_porcelain_blame() {
    let hash = "a".repeat(40);
    let out = format!("{hash} 1 1 2\nauthor Ann\nauthor-time 1000\nsummary first\n\tline one\n{hash} 2 2\n\tline two\n{} 3 3 1\n\tnew\n", "0".repeat(40));
    let got = parse(&out, 1000 + 3 * 86_400);
    assert_eq!(got, [Some("Ann, 3 days ago · first".to_string()), Some("Ann, 3 days ago · first".to_string()), None]);
}
