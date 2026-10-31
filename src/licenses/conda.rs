use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use super::{Dep, Mention, Report, license_file, norm, normalize, npm::read_json, python, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let text = std::fs::read_to_string(manifest).ok()?;
    let own = text
        .lines()
        .find(|l| l.trim_start().starts_with("license"))
        .and_then(|l| normalize(quoted(l).first()?))
        .or_else(|| license_file(dir));
    let mut deps = Vec::new();
    let mut requires = Vec::new();
    let mut names = HashSet::new();
    for env in environments(dir, &text) {
        // Every conda package leaves a record with its license and what it depends on.
        for record in std::fs::read_dir(env.join("conda-meta")).into_iter().flatten().flatten() {
            let Some(meta) = read_json(&record.path()) else { continue };
            let Some(name) = meta["name"].as_str() else { continue };
            if !names.insert(norm(name)) {
                continue;
            }
            let depends = meta["depends"].as_array().into_iter().flatten().filter_map(|d| d.as_str()?.split(' ').next().map(str::to_string));
            requires.push((name.to_string(), depends.collect()));
            let license = meta["license"].as_str().and_then(normalize);
            deps.push(Dep::new(name, meta["version"].as_str().unwrap_or_default(), license));
        }
        // pip installs into the same environment without a conda record.
        let sites = std::fs::read_dir(env.join("lib")).into_iter().flatten().flatten().map(|e| e.path().join("site-packages"));
        for dist in sites.flat_map(|s| python::dist_infos(&s)) {
            if names.insert(norm(&dist.name)) {
                deps.push(dist);
            }
        }
    }
    let roots: Vec<String> = mentions(&text).0.into_iter().map(|m| m.name).collect();
    let mut report = Report::flat(own, deps);
    report.link(&requires, &roots);
    Some(report)
}

/// pixi keeps environments in the project; conda keeps named ones under its install.
fn environments(dir: &Path, manifest: &str) -> Vec<PathBuf> {
    let mut envs: Vec<PathBuf> = std::fs::read_dir(dir.join(".pixi/envs")).into_iter().flatten().flatten().map(|e| e.path()).collect();
    let field = |key: &str| manifest.lines().find_map(|l| l.strip_prefix(key).map(|v| v.trim().to_string()));
    if let Some(prefix) = field("prefix:") {
        envs.push(prefix.into());
    }
    if let Some(name) = field("name:") {
        let home = std::env::home_dir().unwrap_or_default();
        let installs = ["miniconda3", "anaconda3", "miniforge3", "mambaforge", ".conda"].map(|d| home.join(d));
        let installs = installs.into_iter().chain(["/opt/homebrew/Caskroom/miniforge/base".into(), "/opt/miniconda3".into()]);
        envs.extend(installs.map(|i| i.join("envs").join(&name)));
    }
    envs.retain(|e| e.join("conda-meta").is_dir());
    envs
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let mut deps = Vec::new();
    let mut own = None;
    let yaml = !text.lines().any(|l| l.starts_with('['));
    let mut section = String::new();
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if yaml {
            if !raw.starts_with(' ') && !raw.starts_with('-') {
                section = t.to_string();
            } else if section == "dependencies:" && let Some(spec) = t.strip_prefix("- ") {
                let name: String = spec.chars().take_while(|c| c.is_alphanumeric() || "-_.".contains(*c)).collect();
                if !name.is_empty() && name != "pip" && name != "python" {
                    deps.extend(Mention::find(raw, line, &name));
                }
            }
            continue;
        }
        if t.starts_with('[') {
            section = t.trim_matches(['[', ']']).to_string();
            continue;
        }
        let key: String = t.chars().take_while(|c| c.is_alphanumeric() || "-_.".contains(*c)).collect();
        if key.is_empty() || !t[key.len()..].trim_start().starts_with('=') {
            continue;
        }
        if section.ends_with("dependencies") && key != "python" {
            deps.extend(Mention::find(raw, line, &key));
        } else if (section == "project" || section == "workspace") && key == "license" {
            own = Mention::find(raw, line, &key);
        }
    }
    (deps, own)
}

#[std::prelude::rust_2024::test]
fn reads_conda_environments() {
    let pixi = "[workspace]\nname = \"app\"\nlicense = \"MIT\"\n\n[dependencies]\nnumpy = \">=1.26\"\npython = \"3.12.*\"\n\n[pypi-dependencies]\nrich = \"*\"\n";
    let (deps, own) = mentions(pixi);
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("numpy", 5), ("rich", 9)]);
    assert_eq!(own.map(|m| m.line), Some(2));
    let yml = "name: science\ndependencies:\n  - numpy=1.26\n  - pip\n  - pip:\n    - rich==13\n";
    // pip's sublist counts too: those packages ship in the same environment.
    assert_eq!(mentions(yml).0.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["numpy", "rich"]);

    let dir = super::scratch("pixi", &[
        ("pixi.toml", pixi),
        (".pixi/envs/default/conda-meta/numpy-1.26.4-py312.json", r#"{"name": "numpy", "version": "1.26.4", "license": "BSD-3-Clause", "depends": ["libblas >=3.9", "python >=3.12"]}"#),
        (".pixi/envs/default/conda-meta/libblas-3.9.0.json", r#"{"name": "libblas", "version": "3.9.0", "license": "BSD-3-Clause", "depends": []}"#),
        (".pixi/envs/default/lib/python3.12/site-packages/rich-13.0.dist-info/METADATA", "Name: rich\nVersion: 13.0\nLicense-Expression: MIT\n\n"),
    ]);
    let report = report(&dir.join("pixi.toml")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let mut got: Vec<_> = report.deps.iter().map(|d| (d.name.as_str(), d.license.as_deref())).collect();
    got.sort();
    assert_eq!(got, [("libblas", Some("BSD-3-Clause")), ("numpy", Some("BSD-3-Clause")), ("rich", Some("MIT"))]);
    let numpy = report.deps.iter().position(|d| d.name == "numpy").unwrap();
    let libblas = report.deps.iter().position(|d| d.name == "libblas").unwrap();
    assert!(report.edges.contains(&(numpy, libblas)));
    assert_eq!(report.roots.len(), 2);
}
