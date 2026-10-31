use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
};

use serde_json::Value;

use super::{Dep, Mention, Report, license_file, norm, normalize, quoted, spdx_from_name};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let text = std::fs::read_to_string(manifest).ok()?;
    let own = own_license(&text).or_else(|| license_file(dir));
    let installed: HashMap<String, Dep> = environments(dir, &text)
        .iter()
        .flat_map(|sp| dist_infos(sp))
        .map(|d| (norm(&d.name), d))
        .collect();
    // A lockfile names every package and which ones only development needs.
    let locked = uv_lock(dir).or_else(|| poetry_lock(dir)).or_else(|| pipfile_lock(dir));
    let (deps, requires, mut roots) = match locked {
        Some(locked) => {
            let deps = locked
                .packages
                .into_iter()
                .map(|(name, version, shipped)| match installed.get(&norm(&name)) {
                    Some(d) => Dep { shipped, version, aliases: d.aliases.clone(), ..Dep::new(&d.name, "", d.license.clone()) },
                    None => Dep { shipped, ..Dep::new(name, version, None) },
                })
                .collect();
            (deps, locked.requires, locked.roots)
        }
        None => {
            // shortcut: without a lockfile only directly declared dev dependencies count as dev.
            let dev = dev_names(&text);
            let deps = installed.into_values().map(|d| Dep { shipped: !dev.contains(&norm(&d.name)), ..d }).collect();
            (deps, Vec::new(), Vec::new())
        }
    };
    if roots.is_empty() {
        roots = mentions(&text).0.into_iter().map(|m| m.name).collect();
    }
    let mut report = Report::flat(own, deps);
    report.link(&requires, &roots);
    Some(report)
}

/// Site-packages of the project's environment, wherever its tool keeps it:
/// in the project (venv, uv, pdm, rye), or in a per-user cache (Poetry, Pipenv, Hatch).
fn environments(dir: &Path, manifest: &str) -> Vec<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    let mut envs: Vec<PathBuf> = [".venv", "venv", "env"].iter().map(|v| dir.join(v)).collect();
    let name = project_name(manifest).unwrap_or_else(|| dir.file_name().unwrap_or_default().to_string_lossy().into_owned());
    let cached = [
        home.join("Library/Caches/pypoetry/virtualenvs"),
        home.join(".cache/pypoetry/virtualenvs"),
        home.join(".local/share/virtualenvs"),
        home.join("Library/Application Support/hatch/env/virtual").join(&name),
    ];
    for root in cached {
        // These tools name an environment after the project plus a hash.
        let prefix = norm(&name);
        let found = std::fs::read_dir(&root).into_iter().flatten().flatten().map(|e| e.path());
        envs.extend(found.filter(|p| norm(&p.file_name().unwrap_or_default().to_string_lossy()).starts_with(&prefix)));
    }
    let mut sites: Vec<PathBuf> = envs
        .iter()
        .flat_map(|env| std::fs::read_dir(env.join("lib")).into_iter().flatten().flatten().map(|e| e.path().join("site-packages")))
        .collect();
    // PEP 582, as pdm writes it: __pypackages__/<version>/lib.
    sites.extend(std::fs::read_dir(dir.join("__pypackages__")).into_iter().flatten().flatten().map(|e| e.path().join("lib")));
    sites.retain(|p| p.is_dir());
    sites
}

fn project_name(manifest: &str) -> Option<String> {
    let line = manifest.lines().find(|l| l.trim_start().starts_with("name") && l.contains('='))?;
    quoted(line).first().map(|s| s.to_string())
}

pub(super) fn dist_infos(site: &Path) -> Vec<Dep> {
    let Ok(entries) = std::fs::read_dir(site) else { return Vec::new() };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "dist-info"))
        .filter_map(|info| {
            let meta = std::fs::read_to_string(info.join("METADATA")).ok()?;
            let mut dep = metadata(&meta)?;
            dep.aliases = modules(&info);
            Some(dep)
        })
        .collect()
}

/// The import names a distribution provides. Many modern wheels have no top_level.txt,
/// but every one lists its installed files in RECORD.
fn modules(info: &Path) -> Vec<String> {
    if let Ok(top) = std::fs::read_to_string(info.join("top_level.txt")) {
        return top.lines().map(str::to_string).filter(|l| !l.is_empty()).collect();
    }
    let record = std::fs::read_to_string(info.join("RECORD")).unwrap_or_default();
    let mut found: Vec<String> = record
        .lines()
        .filter_map(|l| l.split(',').next()?.split('/').next())
        .filter(|first| !first.ends_with(".dist-info") && !first.ends_with(".data") && !first.starts_with("..") && !first.starts_with("__"))
        .map(|first| first.trim_end_matches(".py").split('.').next().unwrap_or(first).to_string())
        .collect();
    found.sort();
    found.dedup();
    found
}

/// Core metadata headers: the SPDX expression first, then classifiers, then the free-form field.
fn metadata(text: &str) -> Option<Dep> {
    let header = |key: &str| text.lines().take_while(|l| !l.is_empty()).find_map(|l| l.strip_prefix(key)?.strip_prefix(": "));
    let classifiers: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("Classifier: License :: "))
        .filter_map(|c| spdx_from_name(c.rsplit(" :: ").next()?))
        .collect();
    let license = header("License-Expression")
        .and_then(normalize)
        .or_else(|| (!classifiers.is_empty()).then(|| classifiers.join(" OR ")))
        .or_else(|| header("License").and_then(normalize));
    Some(Dep::new(header("Name")?, header("Version").unwrap_or_default(), license))
}

fn own_license(manifest: &str) -> Option<String> {
    let line = manifest.lines().find(|l| l.trim_start().starts_with("license"))?;
    if line.contains("file") {
        return None;
    }
    normalize(quoted(line).last()?)
}

/// Names declared only for development: dependency groups and the tools' dev lists.
fn dev_names(manifest: &str) -> HashSet<String> {
    let mut dev = HashSet::new();
    let mut section = "";
    for raw in manifest.lines() {
        let t = raw.trim();
        if t.starts_with('[') {
            section = t;
            continue;
        }
        let in_dev = section == "[dependency-groups]"
            || section == "[dev-packages]"
            || section.contains("dev-dependencies")
            || section.starts_with("[tool.poetry.group.");
        let tool_dev = t.starts_with("dev-dependencies");
        if in_dev || tool_dev {
            for spec in quoted(t) {
                dev.insert(norm(requirement(spec)));
            }
            let key = requirement(t);
            if !key.is_empty() && t[key.len()..].trim_start().starts_with('=') {
                dev.insert(norm(key));
            }
        }
    }
    dev
}

/// uv.lock: the project is the package with an editable or virtual source.
fn uv_lock(dir: &Path) -> Option<Locked> {
    let text = std::fs::read_to_string(dir.join("uv.lock")).ok()?;
    let mut packages: Vec<(String, String, Vec<String>, Vec<String>, bool)> = Vec::new();
    let mut list = "";
    for raw in text.lines() {
        let t = raw.trim();
        if t == "[[package]]" {
            packages.push(Default::default());
            list = "";
            continue;
        }
        let Some(p) = packages.last_mut() else { continue };
        if t.starts_with('[') {
            list = if t.contains("dev-dependencies") { "dev" } else if t.contains("optional-dependencies") { "deps" } else { "" };
            continue;
        }
        if let Some(v) = t.strip_prefix("name = ") {
            p.0 = v.trim_matches('"').to_string();
        } else if let Some(v) = t.strip_prefix("version = ") {
            p.1 = v.trim_matches('"').to_string();
        } else if t.starts_with("source = ") && (t.contains("editable") || t.contains("virtual")) {
            p.4 = true;
        } else if t.starts_with("dependencies = [") {
            list = "deps";
        }
        // Entries look like `{ name = "requests" },` or `dev = [{ name = "pytest" }]`.
        for entry in t.split("name = ").skip(1) {
            let name = quoted(entry).first().map(|s| s.to_string()).unwrap_or_default();
            match list {
                "deps" => p.2.push(name),
                "dev" => p.3.push(name),
                _ => {}
            }
        }
        if t.ends_with(']') && list == "deps" && !t.starts_with('[') {
            list = "";
        }
    }
    let by_name: HashMap<String, usize> = packages.iter().enumerate().map(|(i, p)| (norm(&p.0), i)).collect();
    let reach = |roots: Vec<String>| {
        let mut seen = HashSet::new();
        let mut queue: VecDeque<String> = roots.into();
        while let Some(name) = queue.pop_front() {
            if let Some(&i) = by_name.get(&norm(&name))
                && seen.insert(i)
            {
                queue.extend(packages[i].2.iter().cloned());
            }
        }
        seen
    };
    let projects: Vec<&(String, String, Vec<String>, Vec<String>, bool)> = packages.iter().filter(|p| p.4).collect();
    let roots: Vec<String> = projects.iter().flat_map(|p| [p.2.clone(), p.3.clone()].concat()).collect();
    let shipped = reach(projects.iter().flat_map(|p| p.2.clone()).collect());
    let all = reach(roots.clone());
    Some(Locked {
        packages: all
            .into_iter()
            .filter(|&i| !packages[i].4)
            .map(|i| (packages[i].0.clone(), packages[i].1.clone(), shipped.contains(&i)))
            .collect(),
        requires: packages.iter().filter(|p| !p.4).map(|p| (p.0.clone(), p.2.clone())).collect(),
        roots,
    })
}

/// A lockfile's packages (name, version, shipped), what each requires, and what the project requires.
#[derive(Debug, PartialEq)]
struct Locked {
    packages: Vec<(String, String, bool)>,
    requires: Vec<(String, Vec<String>)>,
    roots: Vec<String>,
}

/// poetry.lock marks each package with the groups that need it ("main" ships).
fn poetry_lock(dir: &Path) -> Option<Locked> {
    let text = std::fs::read_to_string(dir.join("poetry.lock")).ok()?;
    let mut out: Vec<(String, String, bool)> = Vec::new();
    let mut requires: Vec<(String, Vec<String>)> = Vec::new();
    let mut section = "";
    for raw in text.lines() {
        let t = raw.trim();
        if t == "[[package]]" {
            out.push((String::new(), String::new(), true));
            requires.push(Default::default());
            section = "package";
        } else if t.starts_with('[') {
            section = if t == "[package.dependencies]" { "requires" } else { "" };
        } else if let (Some(p), Some(r)) = (out.last_mut(), requires.last_mut()) {
            match section {
                "package" => {
                    if let Some(v) = t.strip_prefix("name = ") {
                        p.0 = v.trim_matches('"').to_string();
                        r.0 = p.0.clone();
                    } else if let Some(v) = t.strip_prefix("version = ") {
                        p.1 = v.trim_matches('"').to_string();
                    } else if let Some(v) = t.strip_prefix("groups = ") {
                        p.2 = quoted(v).contains(&"main");
                    } else if let Some(v) = t.strip_prefix("category = ") {
                        // Poetry before 1.5.
                        p.2 = v.trim_matches('"') == "main";
                    }
                }
                "requires" if !requirement(t).is_empty() => r.1.push(requirement(t).to_string()),
                _ => {}
            }
        }
    }
    Some(Locked { packages: out, requires, roots: Vec::new() })
}

fn pipfile_lock(dir: &Path) -> Option<Locked> {
    let lock: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("Pipfile.lock")).ok()?).ok()?;
    let entries = |key: &str, shipped: bool| {
        lock[key]
            .as_object()
            .into_iter()
            .flatten()
            .map(move |(name, v)| (name.clone(), v["version"].as_str().unwrap_or_default().trim_start_matches("==").to_string(), shipped))
            .collect::<Vec<_>>()
    };
    Some(Locked { packages: [entries("default", true), entries("develop", false)].concat(), requires: Vec::new(), roots: Vec::new() })
}

fn requirement(spec: &str) -> &str {
    let end = spec.find(|c: char| !(c.is_alphanumeric() || "._-".contains(c))).unwrap_or(spec.len());
    &spec[..end]
}

/// pyproject.toml, Pipfile and requirements.txt.
pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let toml = text.lines().any(|l| l.starts_with('['));
    let mut deps = Vec::new();
    let mut own = None;
    let mut section = String::new();
    let mut in_list = false;
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !toml {
            if !t.starts_with('-') {
                deps.extend(Mention::find(raw, line, requirement(t)).filter(|m| !m.name.is_empty()));
            }
            continue;
        }
        if t.starts_with('[') {
            section = t.trim_matches(['[', ']']).to_string();
            in_list = false;
            continue;
        }
        let key = requirement(t);
        let keyed = section == "packages"
            || section == "dev-packages"
            || section.starts_with("tool.poetry") && section.ends_with("dependencies");
        let listed = section == "project.optional-dependencies" || section == "dependency-groups";
        if (section == "project" && key == "dependencies") || key == "dev-dependencies" {
            in_list = true;
        }
        if keyed && !key.is_empty() && key != "python" && t[key.len()..].trim_start().starts_with('=') {
            deps.extend(Mention::find(raw, line, key));
        } else if in_list || listed {
            for spec in quoted(t) {
                deps.extend(Mention::find(raw, line, requirement(spec)).filter(|m| !m.name.is_empty()));
            }
        } else if section == "project" && key == "license" {
            own = Mention::find(raw, line, key);
        }
        if in_list && t.contains(']') {
            in_list = false;
        }
    }
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start();
    let module = t.strip_prefix("from ").or_else(|| t.strip_prefix("import "))?;
    let first: String = module.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    (!first.is_empty()).then_some(first)
}

#[std::prelude::rust_2024::test]
fn finds_distributions() {
    let pyproject = "[project]\nname = \"x\"\nlicense = \"MIT\"\ndependencies = [\n  \"requests>=2\",\n  \"PyYAML\",\n]\n\n[project.optional-dependencies]\ndev = [\"pytest\"]\n\n[tool.ruff]\nline-length = 100\n";
    let (deps, own) = mentions(pyproject);
    let names: Vec<_> = deps.iter().map(|m| (m.name.as_str(), m.line)).collect();
    assert_eq!(names, [("requests", 4), ("PyYAML", 5), ("pytest", 9)]);
    assert_eq!(own.map(|m| m.line), Some(2));
    let (deps, _) = mentions("# pinned\nflask==3.0\n-r other.txt\nnumpy\n");
    assert_eq!(deps.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["flask", "numpy"]);
    assert_eq!(import("from yaml import safe_load").as_deref(), Some("yaml"));
    assert_eq!(import("import numpy as np").as_deref(), Some("numpy"));
    assert_eq!(import("from . import x"), None);
}

#[std::prelude::rust_2024::test]
fn reads_environment_and_lockfiles() {
    let site = "venv/lib/python3.12/site-packages";
    let dir = super::scratch("py", &[
        ("pyproject.toml", "[project]\nname = \"app\"\nlicense = { text = \"Apache-2.0\" }\n"),
        (&format!("{site}/PyYAML-6.0.dist-info/METADATA"), "Name: PyYAML\nVersion: 6.0\nLicense: MIT\n\nbody"),
        (&format!("{site}/PyYAML-6.0.dist-info/top_level.txt"), "_yaml\nyaml\n"),
        (&format!("{site}/foo-1.dist-info/METADATA"), "Name: foo\nVersion: 1\nClassifier: License :: OSI Approved :: BSD License\n\n"),
        // No top_level.txt: the modules come from RECORD.
        (&format!("{site}/foo-1.dist-info/RECORD"), "foo_core/__init__.py,sha256=x,1\nfoo-1.dist-info/METADATA,,\n"),
        (&format!("{site}/pytest-8.dist-info/METADATA"), "Name: pytest\nVersion: 8.0\nLicense-Expression: MIT\n\n"),
    ]);
    let report1 = report(&dir.join("pyproject.toml")).unwrap();
    assert_eq!(report1.own.as_deref(), Some("Apache-2.0"));
    let yaml = super::find(&report1.deps, "yaml").unwrap();
    assert_eq!((yaml.name.as_str(), yaml.license.as_deref()), ("PyYAML", Some("MIT")));
    assert_eq!(super::find(&report1.deps, "foo_core").unwrap().license.as_deref(), Some("BSD-3-Clause"));

    let uv = "version = 1\n\n[[package]]\nname = \"app\"\nversion = \"0.1.0\"\nsource = { editable = \".\" }\ndependencies = [\n    { name = \"pyyaml\" },\n]\n\n[package.dev-dependencies]\ndev = [\n    { name = \"pytest\" },\n]\n\n[[package]]\nname = \"pyyaml\"\nversion = \"6.0\"\nsource = { registry = \"https://pypi.org/simple\" }\n\n[[package]]\nname = \"pytest\"\nversion = \"8.0\"\nsource = { registry = \"https://pypi.org/simple\" }\ndependencies = [\n    { name = \"foo\" },\n]\n\n[[package]]\nname = \"foo\"\nversion = \"1\"\n";
    std::fs::write(dir.join("uv.lock"), uv).unwrap();
    let report2 = report(&dir.join("pyproject.toml")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let mut got: Vec<_> = report2.deps.iter().map(|d| (norm(&d.name), d.shipped, d.license.clone())).collect();
    got.sort();
    let e = |n: &str, s, l: &str| (n.to_string(), s, Some(l.to_string()));
    assert_eq!(got, [e("foo", false, "BSD-3-Clause"), e("pytest", false, "MIT"), e("pyyaml", true, "MIT")]);
}

#[std::prelude::rust_2024::test]
fn reads_poetry_and_pipenv_locks() {
    let dir = super::scratch("pylock", &[
        ("poetry.lock", "[[package]]\nname = \"requests\"\nversion = \"2.31.0\"\ngroups = [\"main\"]\n\n[package.dependencies]\ncertifi = \">=2017\"\n\n[[package]]\nname = \"pytest\"\nversion = \"8.0\"\ngroups = [\"dev\"]\n"),
        ("Pipfile.lock", r#"{"default": {"flask": {"version": "==3.0.0"}}, "develop": {"black": {"version": "==24.1"}}}"#),
    ]);
    let poetry = poetry_lock(&dir).unwrap();
    let pipenv = pipfile_lock(&dir).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let s = |n: &str, v: &str, b| (n.to_string(), v.to_string(), b);
    assert_eq!(poetry.packages, [s("requests", "2.31.0", true), s("pytest", "8.0", false)]);
    assert_eq!(poetry.requires[0], ("requests".to_string(), vec!["certifi".to_string()]));
    assert_eq!(pipenv.packages, [s("flask", "3.0.0", true), s("black", "24.1", false)]);
}
