use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};

use serde_json::Value;

use super::{Dep, Mention, Report};

pub fn report(manifest: &Path) -> Option<Report> {
    let out = crate::shell::command("cargo")
        .args(["metadata", "--format-version", "1", "--manifest-path"])
        .arg(manifest)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let meta: Value = serde_json::from_slice(&out.stdout).ok()?;
    let packages: HashMap<&str, &Value> =
        meta["packages"].as_array()?.iter().filter_map(|p| Some((p["id"].as_str()?, p))).collect();
    let members: HashSet<&str> = meta["workspace_members"].as_array()?.iter().filter_map(Value::as_str).collect();
    let manifest = manifest.canonicalize().ok()?;
    let own_id = packages
        .iter()
        .find(|(_, p)| p["manifest_path"].as_str().map(Path::new) == Some(&manifest))
        .map(|(id, _)| *id);
    // A virtual manifest has no package of its own; its members stand in for it.
    let roots: Vec<&str> = match own_id {
        Some(id) => vec![id],
        None => members.iter().copied().collect(),
    };
    let own = roots.iter().find_map(|id| packages[id]["license"].as_str()).map(str::to_string);

    let mut edges: HashMap<&str, Vec<(&str, bool)>> = HashMap::new();
    for node in meta["resolve"]["nodes"].as_array()? {
        let deps = node["deps"].as_array().into_iter().flatten().filter_map(|d| {
            let normal = d["dep_kinds"].as_array()?.iter().any(|k| k["kind"].is_null());
            Some((d["pkg"].as_str()?, normal))
        });
        edges.insert(node["id"].as_str()?, deps.collect());
    }
    let shipped = reach(&roots, &edges, true);
    let ids: Vec<&str> = reach(&roots, &edges, false).into_iter().filter(|id| !members.contains(id)).collect();
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(ix, id)| (*id, ix)).collect();
    let deps = ids
        .iter()
        .map(|id| {
            let p = packages[id];
            let name = p["name"].as_str().unwrap_or_default();
            Dep {
                shipped: shipped.contains(id),
                ..Dep::new(name, p["version"].as_str().unwrap_or_default(), p["license"].as_str().map(str::to_string))
            }
        })
        .collect();
    let links = ids
        .iter()
        .flat_map(|id| edges.get(id).into_iter().flatten().filter_map(|(to, _)| Some((index[id], *index.get(to)?))))
        .collect();
    let direct = roots.iter().flat_map(|r| edges.get(r).into_iter().flatten()).filter_map(|(to, _)| index.get(to).copied());
    Some(Report { own, deps, edges: links, roots: direct.collect() })
}

fn reach<'a>(roots: &[&'a str], edges: &HashMap<&'a str, Vec<(&'a str, bool)>>, normal_only: bool) -> HashSet<&'a str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut queue: VecDeque<&str> = roots.iter().copied().collect();
    while let Some(id) = queue.pop_front() {
        for &(dep, normal) in edges.get(id).into_iter().flatten() {
            if (normal || !normal_only) && seen.insert(dep) {
                queue.push_back(dep);
            }
        }
    }
    seen
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let mut deps = Vec::new();
    let mut own = None;
    let mut section = "";
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        // shortcut: `[dependencies.name]` tables are not picked up; handle them if a manifest uses that form.
        if let Some(header) = t.strip_prefix('[') {
            let header = header.trim_matches(|c| c == '[' || c == ']').trim();
            section = if header.ends_with("dependencies") || header == "package" || header == "workspace.package" {
                header
            } else {
                ""
            };
            continue;
        }
        let key: String = t.chars().take_while(|c| c.is_alphanumeric() || "-_".contains(*c)).collect();
        let after = t[key.len()..].trim_start();
        if key.is_empty() || !(after.starts_with('=') || after.starts_with('.')) {
            continue;
        }
        if section.ends_with("dependencies") {
            // `alias = { package = "real-name" }` points at another crate.
            let name = t.split("package").nth(1).and_then(|s| s.split('"').nth(1)).unwrap_or(&key);
            deps.extend(Mention::named(raw, line, &key, name));
        } else if !section.is_empty() && key == "license" {
            own = Mention::find(raw, line, &key);
        }
    }
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start();
    let t = t.strip_prefix("pub ").or_else(|| t.strip_prefix("pub(crate) ")).unwrap_or(t);
    let path = t.strip_prefix("use ").or_else(|| t.strip_prefix("extern crate "))?;
    let first: String = path.trim_start_matches("::").chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!matches!(first.as_str(), "" | "crate" | "self" | "super" | "std" | "core" | "alloc")).then_some(first)
}

#[std::prelude::rust_2024::test]
fn finds_crates() {
    let cargo = "[package]\nname = \"x\"\nlicense = \"MIT\"\n\n[dependencies]\nserde = \"1\"\nfoo = { package = \"real-foo\", version = \"1\" }\nbar.workspace = true\n\n[dev-dependencies]\nbaz = \"1\"\n\n[features]\nqux = []\n";
    let (deps, own) = mentions(cargo);
    let names: Vec<_> = deps.iter().map(|m| (m.name.as_str(), m.line, m.cols.clone())).collect();
    assert_eq!(names, [("serde", 5, 0..5), ("real-foo", 6, 0..3), ("bar", 7, 0..3), ("baz", 10, 0..3)]);
    assert_eq!(own.map(|m| m.line), Some(2));
    assert_eq!(import("use serde_json::Value;").as_deref(), Some("serde_json"));
    assert_eq!(import("pub use ::tokio::{self};").as_deref(), Some("tokio"));
    assert_eq!(import("use crate::git;"), None);
    assert_eq!(import("use std::fs;"), None);
}

#[std::prelude::rust_2024::test]
fn reads_this_crate() {
    let report = report(&Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).expect("cargo metadata");
    assert_eq!(report.own.as_deref(), Some("GPL-3.0-or-later"));
    let serde = report.deps.iter().find(|d| d.name == "serde_json").unwrap();
    assert!(serde.shipped && serde.license.is_some());
    let conflicts: Vec<_> = report
        .deps
        .iter()
        .filter(|d| d.shipped && matches!(super::verdict(report.own.as_deref(), d.license.as_deref()), super::Verdict::Conflict(_)))
        .map(|d| format!("{} {:?}", d.name, d.license))
        .collect();
    assert_eq!(conflicts, Vec::<String>::new());
}
