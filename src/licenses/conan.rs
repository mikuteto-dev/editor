use std::{collections::HashMap, path::Path};

use super::{Dep, Mention, Report, license_file, normalize, quoted};

/// "name/version" references and whether each only builds the package (tool requires).
fn references(text: &str) -> Vec<(usize, String, String, bool)> {
    let mut out = Vec::new();
    let mut section = "";
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if t.starts_with('[') {
            section = t;
            continue;
        }
        let tool = section.contains("tool_requires") || section.contains("build_requires") || t.contains("tool_requires") || t.contains("build_requires");
        let candidates: Vec<&str> = if section == "[requires]" || tool && section.starts_with('[') {
            vec![t]
        } else if t.contains("requires") {
            quoted(t)
        } else {
            Vec::new()
        };
        for spec in candidates {
            if let Some((name, version)) = spec.split('@').next().and_then(|s| s.split_once('/')) {
                let ok = !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || "-_.+".contains(c));
                if ok {
                    out.push((line, name.to_string(), version.split(['[', ' ', '#']).next().unwrap_or(version).to_string(), tool));
                }
            }
        }
    }
    out
}

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let text = std::fs::read_to_string(manifest).ok()?;
    let recipes = recipes();
    let deps = references(&text)
        .into_iter()
        .map(|(_, name, version, tool)| {
            let license = recipes.get(&name).cloned().flatten();
            Dep { shipped: !tool, ..Dep::new(name, version, license) }
        })
        .collect();
    let own = recipe_license(&text).or_else(|| license_file(dir));
    Some(Report::flat(own, deps))
}

/// Licenses of the recipes in the local Conan cache, by package name.
fn recipes() -> HashMap<String, Option<String>> {
    let Some(home) = std::env::home_dir() else { return HashMap::new() };
    // Conan 2 keeps each exported recipe in p/<hash>/e/; Conan 1 in data/<name>/<version>/<user>/<channel>/export/.
    let v2 = std::fs::read_dir(home.join(".conan2/p")).into_iter().flatten().flatten().map(|e| e.path().join("e/conanfile.py"));
    let v1 = std::fs::read_dir(home.join(".conan/data")).into_iter().flatten().flatten().flat_map(|name| {
        std::fs::read_dir(name.path()).into_iter().flatten().flatten().map(|v| v.path().join("_/_/export/conanfile.py"))
    });
    v2.chain(v1)
        .filter_map(|recipe| {
            let text = std::fs::read_to_string(recipe).ok()?;
            let name = text.lines().find(|l| l.trim_start().starts_with("name ="))?;
            Some((quoted(name).first()?.to_string(), recipe_license(&text)))
        })
        .collect()
}

fn recipe_license(recipe: &str) -> Option<String> {
    let line = recipe.lines().find(|l| l.trim_start().starts_with("license"))?;
    // A tuple of licenses means the user may pick.
    let ids: Vec<&str> = quoted(line);
    normalize(&ids.join(" OR "))
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let lines: Vec<&str> = text.lines().collect();
    let deps = references(text).into_iter().filter_map(|(line, name, _, _)| Mention::find(lines[line], line, &name)).collect();
    let own = lines
        .iter()
        .enumerate()
        .find_map(|(line, raw)| raw.trim_start().starts_with("license").then(|| Mention::find(raw, line, "license"))?);
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    super::vcpkg::import(line)
}

#[std::prelude::rust_2024::test]
fn reads_conan_references() {
    let txt = "[requires]\nfmt/10.2.1\nzlib/1.3@user/stable\n\n[tool_requires]\ncmake/3.28.1\n";
    let refs: Vec<_> = references(txt).into_iter().map(|(l, n, v, t)| (l, n, v, t)).collect();
    let r = |l, n: &str, v: &str, t| (l, n.to_string(), v.to_string(), t);
    assert_eq!(refs, [r(1, "fmt", "10.2.1", false), r(2, "zlib", "1.3", false), r(5, "cmake", "3.28.1", true)]);
    let py = "class App(ConanFile):\n    license = \"MIT\"\n    requires = \"spdlog/1.13.0\", \"fmt/10.2.1\"\n\n    def build_requirements(self):\n        self.tool_requires(\"ninja/1.11.1\")\n";
    let refs: Vec<_> = references(py).into_iter().map(|(_, n, _, t)| (n, t)).collect();
    assert_eq!(refs, [("spdlog".to_string(), false), ("fmt".to_string(), false), ("ninja".to_string(), true)]);
    assert_eq!(recipe_license(py).as_deref(), Some("MIT"));
    assert_eq!(recipe_license("    license = (\"MIT\", \"Apache-2.0\")").as_deref(), Some("MIT OR Apache-2.0"));
}
