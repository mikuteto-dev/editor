use std::path::{Path, PathBuf};

use super::{Dep, Mention, Report, license_file, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let text = std::fs::read_to_string(manifest).ok()?;
    let cache = module_cache()?;
    let deps = requires(&text)
        .into_iter()
        .map(|(path, version)| {
            // Go modules declare no license; the LICENSE file in the module cache tells.
            let license = license_file(&cache.join(format!("{}@{version}", escape(&path))));
            Dep::new(path, version, license)
        })
        .collect();
    Some(Report::flat(license_file(dir), deps))
}

fn module_cache() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GOMODCACHE") {
        return Some(dir.into());
    }
    let gopath = std::env::var_os("GOPATH").map(PathBuf::from).or_else(|| Some(std::env::home_dir()?.join("go")))?;
    Some(gopath.join("pkg/mod"))
}

/// The module cache spells capitals as "!" plus the lower-case letter.
fn escape(path: &str) -> String {
    path.chars()
        .flat_map(|c| if c.is_ascii_uppercase() { vec!['!', c.to_ascii_lowercase()] } else { vec![c] })
        .collect()
}

fn requires(text: &str) -> Vec<(String, String)> {
    requirement_lines(text)
        .filter_map(|(_, t)| {
            let mut words = t.split_whitespace();
            Some((words.next()?.to_string(), words.next()?.to_string()))
        })
        .collect()
}

fn requirement_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut block = false;
    text.lines().enumerate().filter_map(move |(line, raw)| {
        let t = raw.trim();
        if t == "require (" {
            block = true;
            return None;
        }
        if block && t == ")" {
            block = false;
            return None;
        }
        let t = if block { t } else { t.strip_prefix("require ")? };
        (!t.is_empty() && !t.starts_with("//")).then_some((line, t))
    })
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let lines: Vec<&str> = text.lines().collect();
    let deps = requirement_lines(text)
        .filter_map(|(line, t)| Mention::find(lines[line], line, t.split_whitespace().next()?))
        .collect();
    (deps, None)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim();
    let t = t.strip_prefix("import ").unwrap_or(t);
    // An import block line is a path, optionally after an alias, and nothing else.
    let words: Vec<&str> = t.split_whitespace().collect();
    if words.is_empty() || words.len() > 2 || !words.last()?.starts_with('"') || !t.ends_with('"') {
        return None;
    }
    let path = *quoted(t).first()?;
    // Standard library paths have no domain.
    path.split('/').next()?.contains('.').then(|| path.to_string())
}

#[std::prelude::rust_2024::test]
fn finds_modules() {
    let gomod = "module example.com/x\n\ngo 1.22\n\nrequire (\n\tgithub.com/BurntSushi/toml v1.3.2\n\tgolang.org/x/sys v0.1.0 // indirect\n)\n\nrequire github.com/a/b v2.0.0\n";
    let (deps, _) = mentions(gomod);
    let names: Vec<_> = deps.iter().map(|m| (m.name.as_str(), m.line, m.cols.clone())).collect();
    assert_eq!(names, [("github.com/BurntSushi/toml", 5, 1..27), ("golang.org/x/sys", 6, 1..17), ("github.com/a/b", 9, 8..22)]);
    assert_eq!(escape("github.com/BurntSushi/toml"), "github.com/!burnt!sushi/toml");
    assert_eq!(import("\t\"github.com/a/b/pkg\"").as_deref(), Some("github.com/a/b/pkg"));
    assert_eq!(import("import yaml \"gopkg.in/yaml.v3\"").as_deref(), Some("gopkg.in/yaml.v3"));
    assert_eq!(import("\t\"fmt\""), None);
    assert_eq!(import("\tx := \"github.com/a\""), None);
}
