mod lock;

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use serde_json::Value;

pub(super) use lock::split_spec;
use lock::{bun_lock, package_lock, pnpm_lock, yarn_lock};

use super::{Dep, Mention, Report, normalize, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let pkg: Value = read_json(manifest)?;
    let own = license_of(&pkg);
    if let Some(lock) = read_json(&dir.join("package-lock.json"))
        && let Some(entries) = lock["packages"].as_object()
    {
        let graph = package_lock(entries);
        return Some(graph.into_report(own, |key, _, _| license_of(&entries[key])));
    }
    let installed = index(dir);
    let graph = bun_lock(dir)
        .or_else(|| pnpm_lock(dir))
        .or_else(|| yarn_lock(dir, &pkg));
    let Some(graph) = graph else {
        // shortcut: with no lockfile, only direct devDependencies count as dev.
        let dev: HashSet<&str> = pkg["devDependencies"].as_object().into_iter().flatten().map(|(k, _)| k.as_str()).collect();
        let deps = installed
            .into_iter()
            .map(|((name, version), license)| Dep { shipped: !dev.contains(name.as_str()), ..Dep::new(name, version, license) })
            .collect();
        return Some(Report::flat(own, deps));
    };
    let berry = dir.join(".yarn/cache");
    Some(graph.into_report(own, |_, name, version| match installed.get(&(name.to_string(), version.to_string())) {
        Some(license) => license.clone(),
        // Yarn's Plug'n'Play keeps packages zipped instead of in node_modules.
        None => zipped(&berry, name, version).as_ref().and_then(license_of),
    }))
}

fn index(dir: &Path) -> HashMap<(String, String), Option<String>> {
    let mut found = HashMap::new();
    walk(&dir.join("node_modules"), &mut found, 0);
    found
}

fn walk(modules: &Path, found: &mut HashMap<(String, String), Option<String>>, depth: u32) {
    let Ok(entries) = std::fs::read_dir(modules) else { return };
    if depth > 8 {
        return;
    }
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // pnpm links the top level into its store; reading the store once covers both.
        if entry.file_type().is_ok_and(|t| t.is_symlink()) || name == ".bin" || name == ".cache" {
            continue;
        }
        if name == ".pnpm" {
            for stored in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                walk(&stored.path().join("node_modules"), found, depth + 1);
            }
        } else if name.starts_with('@') {
            walk(&path, found, depth);
        } else if let Some(p) = read_json(&path.join("package.json")) {
            if let (Some(n), Some(v)) = (p["name"].as_str(), p["version"].as_str()) {
                found.entry((n.to_string(), v.to_string())).or_insert_with(|| license_of(&p));
            }
            walk(&path.join("node_modules"), found, depth + 1);
        }
    }
}

fn zipped(cache: &Path, name: &str, version: &str) -> Option<Value> {
    let prefix = format!("{}-npm-{version}-", name.replace('/', "-"));
    let zip = std::fs::read_dir(cache).ok()?.flatten().find(|e| e.file_name().to_string_lossy().starts_with(&prefix))?;
    let out = std::process::Command::new("unzip")
        .arg("-p")
        .arg(zip.path())
        .arg(format!("node_modules/{name}/package.json"))
        .output()
        .ok()?;
    serde_json::from_slice(&out.stdout).ok()
}

pub(super) fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub(super) fn license_of(pkg: &Value) -> Option<String> {
    match &pkg["license"] {
        Value::String(s) => normalize(s),
        // Old packages wrote {"type": "MIT"} or a "licenses" list.
        Value::Object(o) => normalize(o.get("type")?.as_str()?),
        _ => {
            let list: Vec<&str> = pkg["licenses"].as_array()?.iter().filter_map(|l| l["type"].as_str()).collect();
            normalize(&list.join(" OR "))
        }
    }
}

pub(super) fn json_mentions(text: &str, sections: &[&str]) -> (Vec<Mention>, Option<Mention>) {
    let mut deps = Vec::new();
    let mut own = None;
    let mut inside = false;
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        let key = t.strip_prefix('"').and_then(|r| r.split('"').next()).unwrap_or_default();
        if t.starts_with('}') {
            inside = false;
        } else if t.ends_with('{') {
            inside = sections.contains(&key);
        } else if inside && !key.is_empty() {
            deps.extend(Mention::find(raw, line, key));
        } else if key == "license" && raw.len() - raw.trim_start().len() <= 4 {
            own = Mention::find(raw, line, key);
        }
    }
    (deps, own)
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    json_mentions(text, &["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"])
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start();
    let imports = t.starts_with("import ") || t.contains(" from ") || t.contains("require(") || t.contains("import(");
    if !imports || t.starts_with("//") {
        return None;
    }
    package_of(quoted(t).last()?)
}

/// The package a specifier names: "lodash/fp" → lodash, Deno's "jsr:@std/path@^1/join" → @std/path.
pub(super) fn package_of(spec: &str) -> Option<String> {
    let spec = spec.strip_prefix("npm:").or_else(|| spec.strip_prefix("jsr:")).unwrap_or(spec).trim_start_matches('/');
    if spec.starts_with('.') || spec.is_empty() || spec.contains(':') {
        return None;
    }
    let mut parts = spec.split('/');
    let first = parts.next()?;
    let unversioned = |s: &str| s.split('@').next().unwrap_or(s).to_string();
    Some(match first.strip_prefix('@') {
        Some(scope) => format!("@{scope}/{}", unversioned(parts.next()?)),
        None => unversioned(first),
    })
}

#[std::prelude::rust_2024::test]
fn finds_packages() {
    let npm = "{\n  \"name\": \"x\",\n  \"license\": \"MIT\",\n  \"dependencies\": {\n    \"@scope/a\": \"^1\",\n    \"b\": \"2\"\n  },\n  \"scripts\": {\n    \"c\": \"run\"\n  }\n}\n";
    let (deps, own) = mentions(npm);
    let names: Vec<_> = deps.iter().map(|m| (m.name.as_str(), m.line, m.cols.clone())).collect();
    assert_eq!(names, [("@scope/a", 4, 5..13), ("b", 5, 5..6)]);
    assert_eq!(own.map(|m| (m.line, m.cols)), Some((2, 3..10)));
    assert_eq!(import("import React from 'react';").as_deref(), Some("react"));
    assert_eq!(import("import { x } from \"@scope/pkg/sub\";").as_deref(), Some("@scope/pkg"));
    assert_eq!(import("const fs = require('node:fs');"), None);
    assert_eq!(import("import a from './a';"), None);
    assert_eq!(import("export * from \"lodash/fp\";").as_deref(), Some("lodash"));
    assert_eq!(import("import { join } from \"jsr:@std/path@^1.0.8/join\";").as_deref(), Some("@std/path"));
    assert_eq!(import("import chalk from \"npm:chalk@5\";").as_deref(), Some("chalk"));
    assert_eq!(import("import x from \"https://deno.land/x/y.ts\";"), None);
}

#[std::prelude::rust_2024::test]
fn reads_node_modules_without_lockfile() {
    let dir = super::scratch("npm", &[
        ("package.json", r#"{"license": "MIT", "devDependencies": {"jest": "1"}}"#),
        ("node_modules/left-pad/package.json", r#"{"name": "left-pad", "version": "1.3.0", "license": "WTFPL"}"#),
        ("node_modules/jest/package.json", r#"{"name": "jest", "version": "29.0.0", "license": {"type": "MIT"}}"#),
        ("node_modules/@x/y/package.json", r#"{"name": "@x/y", "version": "2.0.0", "licenses": [{"type": "Apache 2.0"}]}"#),
    ]);
    let report = report(&dir.join("package.json")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let mut got: Vec<_> = report.deps.iter().map(|d| (d.name.clone(), d.shipped, d.license.clone())).collect();
    got.sort();
    let s = |n: &str, shipped, l: &str| (n.to_string(), shipped, Some(l.to_string()));
    assert_eq!(got, [s("@x/y", true, "Apache-2.0"), s("jest", false, "MIT"), s("left-pad", true, "WTFPL")]);
}

/// The same project locked by bun, pnpm and yarn: a shipped package `a` that needs `b`,
/// and a dev tool `t` that needs another version of `b`, nested under it.
#[std::prelude::rust_2024::test]
fn follows_lockfiles_to_tell_shipped_from_dev() {
    let pkg = r#"{"license": "MIT", "dependencies": {"a": "^1"}, "devDependencies": {"t": "^1"}}"#;
    let modules: [(&str, &str); 4] = [
        ("node_modules/a/package.json", r#"{"name": "a", "version": "1.0.0", "license": "MIT"}"#),
        ("node_modules/b/package.json", r#"{"name": "b", "version": "1.0.0", "license": "ISC"}"#),
        ("node_modules/t/package.json", r#"{"name": "t", "version": "1.0.0", "license": "GPL-3.0"}"#),
        ("node_modules/t/node_modules/b/package.json", r#"{"name": "b", "version": "2.0.0", "license": "GPL-3.0"}"#),
    ];
    let bun = r#"{
  "lockfileVersion": 1,
  "workspaces": { "": { "dependencies": { "a": "^1", }, "devDependencies": { "t": "^1", }, }, },
  "packages": {
    "a": ["a@1.0.0", "", { "dependencies": { "b": "^1" } }, "sha"],
    "b": ["b@1.0.0", "", {}, "sha"],
    "t": ["t@1.0.0", "", { "dependencies": { "b": "^2" } }, "sha"],
    "t/b": ["b@2.0.0", "", {}, "sha"],
  }
}"#;
    let pnpm = "lockfileVersion: '9.0'\n\nimporters:\n\n  .:\n    dependencies:\n      a:\n        specifier: ^1\n        version: 1.0.0\n    devDependencies:\n      t:\n        specifier: ^1\n        version: 1.0.0\n\npackages:\n\n  a@1.0.0:\n    resolution: {integrity: sha}\n\nsnapshots:\n\n  a@1.0.0:\n    dependencies:\n      b: 1.0.0\n\n  b@1.0.0: {}\n\n  b@2.0.0: {}\n\n  t@1.0.0:\n    dependencies:\n      b: 2.0.0\n";
    let yarn = "# yarn lockfile v1\n\n\na@^1:\n  version \"1.0.0\"\n  dependencies:\n    b \"^1\"\n\nb@^1:\n  version \"1.0.0\"\n\nb@^2:\n  version \"2.0.0\"\n\nt@^1:\n  version \"1.0.0\"\n  dependencies:\n    b \"^2\"\n";
    for (lock, text) in [("bun.lock", bun), ("pnpm-lock.yaml", pnpm), ("yarn.lock", yarn)] {
        let mut files = vec![("package.json", pkg), (lock, text)];
        files.extend(modules);
        let dir = super::scratch(lock, &files);
        let report = report(&dir.join("package.json")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        let label = |ix: usize| format!("{}@{}", report.deps[ix].name, report.deps[ix].version);
        let mut edges: Vec<_> = report.edges.iter().map(|&(a, b)| (label(a), label(b))).collect();
        edges.sort();
        let e = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(edges, [e("a@1.0.0", "b@1.0.0"), e("t@1.0.0", "b@2.0.0")], "{lock}");
        let mut roots: Vec<_> = report.roots.iter().map(|&r| label(r)).collect();
        roots.sort();
        assert_eq!(roots, ["a@1.0.0", "t@1.0.0"], "{lock}");
        let mut got: Vec<_> = report.deps.iter().map(|d| (d.name.clone(), d.version.clone(), d.shipped, d.license.clone())).collect();
        got.sort();
        let d = |n: &str, v: &str, s, l: &str| (n.to_string(), v.to_string(), s, Some(l.to_string()));
        assert_eq!(
            got,
            [d("a", "1.0.0", true, "MIT"), d("b", "1.0.0", true, "ISC"), d("b", "2.0.0", false, "GPL-3.0"), d("t", "1.0.0", false, "GPL-3.0")],
            "{lock}"
        );
    }
}
