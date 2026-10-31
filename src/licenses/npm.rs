use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};

use serde_json::Value;

use super::{Dep, Mention, Report, jsonc, normalize, quoted};

/// A lockfile as a graph: packages by key, and the keys the project itself depends on.
#[derive(Default)]
struct Graph {
    /// key → (name, version, dependency keys)
    nodes: HashMap<String, (String, String, Vec<String>)>,
    shipped_roots: Vec<String>,
    dev_roots: Vec<String>,
}

impl Graph {
    fn reach(&self, roots: &[String]) -> HashSet<String> {
        let mut seen: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<&String> = roots.iter().collect();
        while let Some(key) = queue.pop_front() {
            if self.nodes.contains_key(key) && seen.insert(key.clone()) {
                queue.extend(&self.nodes[key].2);
            }
        }
        seen
    }

    /// Packages reachable from the project, shipped when reachable without a dev dependency.
    fn into_report(self, own: Option<String>, license: impl Fn(&str, &str, &str) -> Option<String>) -> Report {
        let shipped = self.reach(&self.shipped_roots);
        let mut keys: Vec<String> = self.reach(&[self.shipped_roots.clone(), self.dev_roots.clone()].concat()).into_iter().collect();
        keys.sort();
        let index: HashMap<&str, usize> = keys.iter().enumerate().map(|(ix, k)| (k.as_str(), ix)).collect();
        let deps = keys
            .iter()
            .map(|key| {
                let (name, version, _) = &self.nodes[key];
                Dep { shipped: shipped.contains(key), ..Dep::new(name, version, license(key, name, version)) }
            })
            .collect();
        let edges = keys
            .iter()
            .flat_map(|key| self.nodes[key].2.iter().filter_map(|to| Some((index[key.as_str()], *index.get(to.as_str())?))))
            .collect();
        let roots = self.shipped_roots.iter().chain(&self.dev_roots).filter_map(|k| index.get(k.as_str()).copied()).collect();
        Report { own, deps, edges, roots }
    }
}

const PROD: [&str; 3] = ["dependencies", "optionalDependencies", "peerDependencies"];

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let pkg: Value = read_json(manifest)?;
    let own = license_of(&pkg);
    // The npm lockfile records every installed package with its license.
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

/// package-lock.json keys packages by install path; a dependency resolves to the nearest copy.
fn package_lock(entries: &serde_json::Map<String, Value>) -> Graph {
    let resolve = |from: &str, name: &str| {
        let mut base = from.to_string();
        loop {
            let key = if base.is_empty() { format!("node_modules/{name}") } else { format!("{base}/node_modules/{name}") };
            if entries.contains_key(&key) {
                return Some(key);
            }
            if base.is_empty() {
                return None;
            }
            base = base.rfind("/node_modules/").map_or(String::new(), |i| base[..i].to_string());
        }
    };
    let requires = |p: &Value, kinds: &[&str], from: &str| -> Vec<String> {
        kinds.iter().flat_map(|k| p[k].as_object().into_iter().flatten()).filter_map(|(n, _)| resolve(from, n)).collect()
    };
    let mut graph = Graph::default();
    for (key, p) in entries.iter().filter(|(k, _)| !k.is_empty()) {
        let Some((_, name)) = key.rsplit_once("node_modules/") else { continue };
        let version = p["version"].as_str().unwrap_or_default().to_string();
        graph.nodes.insert(key.clone(), (name.to_string(), version, requires(p, &PROD, key)));
    }
    let root = &entries.get("").cloned().unwrap_or_default();
    graph.shipped_roots = requires(root, &PROD, "");
    graph.dev_roots = requires(root, &["devDependencies"], "");
    graph
}

/// Every package under node_modules, nested and pnpm-stored ones included, by name and version.
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

/// "name@version" with the version after the last "@" that is not the scope's.
pub(super) fn split_spec(spec: &str) -> Option<(&str, &str)> {
    let at = spec.get(1..)?.find('@')? + 1;
    Some((&spec[..at], &spec[at + 1..]))
}

/// bun.lock: packages keyed by install path ("a", "a/b" for b nested under a).
fn bun_lock(dir: &Path) -> Option<Graph> {
    let lock = jsonc(&std::fs::read_to_string(dir.join("bun.lock")).ok()?)?;
    let packages = lock["packages"].as_object()?;
    // A dependency resolves to the nearest copy: nested under the dependent first, then up the tree.
    let resolve = |parent: &str, name: &str| -> Option<String> {
        let mut base = parent.to_string();
        loop {
            let key = if base.is_empty() { name.to_string() } else { format!("{base}/{name}") };
            if packages.contains_key(&key) {
                return Some(key);
            }
            if base.is_empty() {
                return None;
            }
            base = match base.rsplit_once('/') {
                // A scoped parent is two segments.
                Some((rest, _)) if rest.rsplit('/').next().is_some_and(|s| s.starts_with('@')) => {
                    rest.rsplit_once('/').map_or(String::new(), |(r, _)| r.to_string())
                }
                Some((rest, _)) => rest.to_string(),
                None => String::new(),
            };
        }
    };
    let mut graph = Graph::default();
    for (key, entry) in packages {
        let (name, version) = split_spec(entry[0].as_str()?)?;
        let meta = &entry[2];
        let deps = PROD
            .iter()
            .flat_map(|kind| meta[kind].as_object().into_iter().flatten())
            .filter_map(|(dep, _)| resolve(key, dep))
            .collect();
        graph.nodes.insert(key.clone(), (name.to_string(), version.to_string(), deps));
    }
    let root = &lock["workspaces"][""];
    for kind in PROD {
        graph.shipped_roots.extend(root[kind].as_object().into_iter().flatten().filter_map(|(n, _)| resolve("", n)));
    }
    graph.dev_roots.extend(root["devDependencies"].as_object().into_iter().flatten().filter_map(|(n, _)| resolve("", n)));
    Some(graph)
}

/// pnpm-lock.yaml (v6 and v9): the root importer's dependencies, then each package's.
fn pnpm_lock(dir: &Path) -> Option<Graph> {
    let text = std::fs::read_to_string(dir.join("pnpm-lock.yaml")).ok()?;
    let clean = |v: &str| v.trim().trim_matches(['\'', '"']).split('(').next().unwrap_or_default().to_string();
    // v5 wrote "/name/1.0.0", v6 "/name@1.0.0", v9 "name@1.0.0".
    let key_of = |k: &str| {
        // "b@1.0.0: {}" carries its (empty) body on the same line.
        let k = clean(k.split(": ").next().unwrap_or(k).trim_end_matches(':'));
        let k = k.trim_start_matches('/');
        match split_spec(k) {
            Some(_) => k.to_string(),
            None => k.rsplit_once('/').map_or(k.to_string(), |(n, v)| format!("{n}@{v}")),
        }
    };
    let mut graph = Graph::default();
    let mut section = "";
    let mut importer = String::new();
    let mut kind = String::new();
    let mut current = String::new();
    let mut root_dep: Option<(String, bool)> = None;
    for raw in text.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if indent == 0 {
            section = match t.trim_end_matches(':') {
                "importers" => "importers",
                "snapshots" | "packages" => "packages",
                // Lockfiles of a single project list the root's dependencies at the top level.
                k @ ("dependencies" | "devDependencies" | "optionalDependencies") => {
                    importer = ".".into();
                    kind = k.into();
                    "root"
                }
                _ => "",
            };
            continue;
        }
        let (key, value) = t.split_once(':').map_or((t, ""), |(k, v)| (k.trim_matches(['\'', '"']), v.trim()));
        let shift = if section == "root" { 4 } else { 0 };
        match (section, indent + shift) {
            ("importers", 2) => importer = key.to_string(),
            ("importers", 4) => kind = key.to_string(),
            ("importers" | "root", 6) if importer == "." => {
                root_dep = Some((key.to_string(), kind == "devDependencies"));
                // v5 put the version right on the name's line.
                if !value.is_empty() {
                    push_root(&mut graph, key, &clean(value), kind == "devDependencies");
                }
            }
            ("importers" | "root", 8) if key == "version" && importer == "." => {
                if let Some((name, dev)) = root_dep.take() {
                    push_root(&mut graph, &name, &clean(value), dev);
                }
            }
            ("packages", 2) => {
                current = key_of(t);
                let (name, version) = split_spec(&current).map(|(n, v)| (n.to_string(), v.to_string())).unwrap_or_default();
                graph.nodes.entry(current.clone()).or_insert((name, version, Vec::new()));
            }
            ("packages", 4) => kind = key.to_string(),
            ("packages", 6) if PROD.contains(&kind.as_str()) => {
                let dep = format!("{key}@{}", clean(value));
                graph.nodes.entry(current.clone()).or_default().2.push(dep);
            }
            _ => {}
        }
    }
    Some(graph)
}

fn push_root(graph: &mut Graph, name: &str, version: &str, dev: bool) {
    let key = format!("{name}@{version}");
    if dev { graph.dev_roots.push(key) } else { graph.shipped_roots.push(key) }
}

/// yarn.lock, classic and Berry: entries keyed by every range that resolved to them.
fn yarn_lock(dir: &Path, pkg: &Value) -> Option<Graph> {
    let text = std::fs::read_to_string(dir.join("yarn.lock")).ok()?;
    let mut specs: HashMap<String, String> = HashMap::new();
    let mut graph = Graph::default();
    let mut current = String::new();
    let mut in_deps = false;
    let mut pending: Vec<(String, String, String)> = Vec::new();
    for raw in text.lines() {
        let indent = raw.len() - raw.trim_start().len();
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if indent == 0 {
            let header = t.trim_end_matches(':');
            if header == "__metadata" {
                current.clear();
                continue;
            }
            let keys: Vec<String> = header.split(", ").map(|s| s.trim_matches('"').to_string()).collect();
            current = keys[0].clone();
            let name = split_spec(&current).map(|(n, _)| n.to_string()).unwrap_or_default();
            graph.nodes.insert(current.clone(), (name, String::new(), Vec::new()));
            for key in keys {
                specs.insert(key, current.clone());
            }
            in_deps = false;
        } else if current.is_empty() {
            continue;
        } else if indent == 2 {
            let (key, value) = t.split_once([' ', ':']).map_or((t, ""), |(k, v)| (k, v.trim_start_matches(':').trim()));
            in_deps = PROD.contains(&key.trim_end_matches(':'));
            if key == "version" {
                graph.nodes.get_mut(&current).unwrap().1 = value.trim_matches('"').to_string();
            }
        } else if indent == 4 && in_deps {
            let (name, range) = t.split_once([' ', ':']).map_or((t, ""), |(k, v)| (k, v.trim_start_matches(':').trim()));
            pending.push((current.clone(), name.trim_matches('"').to_string(), range.trim_matches('"').to_string()));
        }
    }
    // Berry writes ranges with their protocol ("npm:^1"); classic writes them bare.
    let find = |name: &str, range: &str| {
        specs.get(&format!("{name}@{range}")).or_else(|| specs.get(&format!("{name}@npm:{range}"))).cloned()
    };
    for (from, name, range) in pending {
        if let Some(to) = find(&name, &range) {
            graph.nodes.get_mut(&from).unwrap().2.push(to);
        }
    }
    for kind in PROD {
        let roots = pkg[kind].as_object().into_iter().flatten().filter_map(|(n, r)| find(n, r.as_str()?));
        graph.shipped_roots.extend(roots);
    }
    let dev = pkg["devDependencies"].as_object().into_iter().flatten().filter_map(|(n, r)| find(n, r.as_str()?));
    graph.dev_roots.extend(dev);
    Some(graph)
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

/// Keys inside the named top-level objects of a formatted JSON manifest, and its "license" key.
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

#[cfg(test)]
fn shipped_names(report: &Report) -> Vec<(String, bool, Option<String>)> {
    let mut got: Vec<_> = report.deps.iter().map(|d| (d.name.clone(), d.shipped, d.license.clone())).collect();
    got.sort();
    got
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
    let s = |n: &str, shipped, l: &str| (n.to_string(), shipped, Some(l.to_string()));
    assert_eq!(shipped_names(&report), [s("@x/y", true, "Apache-2.0"), s("jest", false, "MIT"), s("left-pad", true, "WTFPL")]);
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
