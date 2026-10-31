use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
};

use serde_json::Value;

use crate::licenses::{Dep, Report, jsonc};

#[derive(Default)]
pub(super) struct Graph {
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
    pub(super) fn into_report(self, own: Option<String>, license: impl Fn(&str, &str, &str) -> Option<String>) -> Report {
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

/// "name@version" with the version after the last "@" that is not the scope's.
pub(in crate::licenses) fn split_spec(spec: &str) -> Option<(&str, &str)> {
    let at = spec.get(1..)?.find('@')? + 1;
    Some((&spec[..at], &spec[at + 1..]))
}

/// package-lock.json keys packages by install path; a dependency resolves to the nearest copy.
pub(super) fn package_lock(entries: &serde_json::Map<String, Value>) -> Graph {
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

/// bun.lock: packages keyed by install path ("a", "a/b" for b nested under a).
pub(super) fn bun_lock(dir: &Path) -> Option<Graph> {
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

pub(super) fn pnpm_lock(dir: &Path) -> Option<Graph> {
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
pub(super) fn yarn_lock(dir: &Path, pkg: &Value) -> Option<Graph> {
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
