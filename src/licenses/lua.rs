use std::path::{Path, PathBuf};

use super::{Dep, Mention, Report, license_file, normalize, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let text = std::fs::read_to_string(manifest).ok()?;
    let own = rockspec_license(&text).or_else(|| license_file(dir));
    let trees = rock_trees(dir);
    let deps = dependencies(&text)
        .into_iter()
        .map(|(_, name)| {
            let installed = trees.iter().find_map(|tree| installed_rock(&tree.join(&name)));
            match installed {
                Some((version, license, aliases)) => Dep { aliases, ..Dep::new(name, version, license) },
                None => Dep::new(name, "", None),
            }
        })
        .collect();
    Some(Report::flat(own, deps))
}

/// Top-level module names in a rockspec's `build.modules`: luasocket provides "socket".
fn modules(spec: &str) -> Vec<String> {
    let Some(block) = spec.split("modules").nth(1) else { return Vec::new() };
    let mut names: Vec<String> = block
        .lines()
        .take_while(|l| !l.trim_start().starts_with('}'))
        .filter_map(|l| {
            let key = l.trim().split('=').next()?.trim();
            let key = key.trim_start_matches("[\"").trim_end_matches("\"]").trim_matches(['[', ']', '"', '\'']);
            (!key.is_empty() && !key.contains(' ') && !key.contains('{')).then(|| key.split('.').next().unwrap_or(key).to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// LuaRocks trees: the project's own, the user's, then Homebrew's.
fn rock_trees(dir: &Path) -> Vec<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    [dir.join("lua_modules/lib/luarocks"), home.join(".luarocks/lib/luarocks"), "/opt/homebrew/lib/luarocks".into()]
        .iter()
        .flat_map(|root| std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path()))
        .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("rocks-")))
        .collect()
}

fn installed_rock(rock: &Path) -> Option<(String, Option<String>, Vec<String>)> {
    let version = std::fs::read_dir(rock).ok()?.flatten().next()?.path();
    let spec = std::fs::read_dir(&version).ok()?.flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "rockspec"))?;
    let text = std::fs::read_to_string(spec).ok()?;
    Some((version.file_name()?.to_string_lossy().into_owned(), rockspec_license(&text), modules(&text)))
}

fn rockspec_license(text: &str) -> Option<String> {
    let line = text.lines().find(|l| l.trim_start().starts_with("license"))?;
    normalize(quoted(line).first()?)
}

/// Names in the `dependencies = { ... }` table, with their line; Lua itself is not a rock.
fn dependencies(text: &str) -> Vec<(usize, String)> {
    let mut inside = false;
    let mut out = Vec::new();
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if t.starts_with("dependencies") {
            inside = true;
        }
        if inside {
            for spec in quoted(t) {
                let name = spec.split_whitespace().next().unwrap_or_default();
                if !name.is_empty() && name != "lua" {
                    out.push((line, name.to_string()));
                }
            }
            if t.contains('}') {
                inside = false;
            }
        }
    }
    out
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let lines: Vec<&str> = text.lines().collect();
    let deps = dependencies(text).into_iter().filter_map(|(line, name)| Mention::find(lines[line], line, &name)).collect();
    let own = lines
        .iter()
        .enumerate()
        .find_map(|(line, raw)| raw.trim_start().starts_with("license").then(|| Mention::find(raw, line, "license"))?);
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    let rest = line.split("require").nth(1)?;
    let module = *quoted(rest.trim_start().trim_start_matches('(')).first()?;
    Some(module.split('.').next()?.to_string())
}

#[std::prelude::rust_2024::test]
fn finds_rocks() {
    let spec = "package = \"x\"\ndescription = {\n  license = \"MIT\"\n}\ndependencies = {\n  \"lua >= 5.1\",\n  \"lpeg >= 1.0\",\n}\n";
    let (deps, own) = mentions(spec);
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("lpeg", 6)]);
    assert_eq!(own.map(|m| m.line), Some(2));
    assert_eq!(rockspec_license(spec).as_deref(), Some("MIT"));
    assert_eq!(import("local lpeg = require \"lpeg\"").as_deref(), Some("lpeg"));
    assert_eq!(import("local json = require('cjson.safe')").as_deref(), Some("cjson"));
    let socket = "build = {\n  type = \"builtin\",\n  modules = {\n    [\"socket.core\"] = { sources = {} },\n    ltn12 = \"src/ltn12.lua\",\n  }\n}\n";
    assert_eq!(modules(socket), ["ltn12", "socket"]);
}
