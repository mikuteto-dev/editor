use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    Dep, Mention, Report, jsonc, license_file, normalize,
    npm::{license_of, package_of, read_json, split_spec},
    quoted, spdx_from_name,
};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let config = jsonc(&std::fs::read_to_string(manifest).ok()?)?;
    let own = config["license"].as_str().and_then(normalize).or_else(|| license_file(dir));
    let cache = deno_dir()?;
    // deno.lock lists every resolved package, transitive ones included.
    let lock = std::fs::read_to_string(dir.join("deno.lock")).ok().and_then(|t| jsonc(&t)).unwrap_or_default();
    let mut deps = Vec::new();
    for (key, _) in lock["npm"].as_object().into_iter().flatten() {
        let Some((name, version)) = split_spec(key) else { continue };
        // A peer-resolved entry reads "name@1.0.0_peer@2.0.0".
        let version = version.split('_').next().unwrap_or(version);
        let pkg = read_json(&dir.join("node_modules").join(name).join("package.json"))
            .or_else(|| read_json(&cache.join("npm/registry.npmjs.org").join(name).join(version).join("package.json")));
        deps.push(Dep::new(name, version, pkg.as_ref().and_then(license_of)));
    }
    for (key, _) in lock["jsr"].as_object().into_iter().flatten() {
        let Some((name, version)) = split_spec(key) else { continue };
        deps.push(Dep::new(name, version, jsr_license(&cache, name, version)));
    }
    // Without a lockfile, the import map still names the direct dependencies.
    let imports: Vec<String> =
        config["imports"].as_object().into_iter().flatten().filter_map(|(_, v)| package_of(v.as_str()?)).collect();
    if deps.is_empty() {
        deps.extend(imports.iter().map(|name| Dep::new(name, "", None)));
    }
    let names = |list: &Value| -> Vec<String> { list.as_array().into_iter().flatten().filter_map(|s| package_of(s.as_str()?)).collect() };
    let requires: Vec<(String, Vec<String>)> = ["jsr", "npm"]
        .iter()
        .flat_map(|kind| lock[kind].as_object().into_iter().flatten())
        .filter_map(|(key, entry)| Some((split_spec(key)?.0.to_string(), names(&entry["dependencies"]))))
        .collect();
    let roots = names(&lock["workspace"]["dependencies"]);
    let mut report = Report::flat(own, deps);
    report.link(&requires, if roots.is_empty() { &imports } else { &roots });
    Some(report)
}

fn deno_dir() -> Option<PathBuf> {
    std::env::var_os("DENO_DIR")
        .map(PathBuf::from)
        .or_else(|| Some(std::env::home_dir()?.join("Library/Caches/deno")))
}

/// Deno caches only the modules a program loads, under the SHA-256 of each URL's path, and JSR's
/// metadata carries no license; the header comment of the package's entry module states it.
fn jsr_license(cache: &Path, name: &str, version: &str) -> Option<String> {
    use sha2::{Digest, Sha256};
    let root = cache.join("remote/https/jsr.io");
    let read = |path: String| {
        let hash: String = Sha256::digest(path.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        std::fs::read_to_string(root.join(hash)).ok()
    };
    // The cached file appends its own headers after the JSON.
    let meta: Option<Value> = read(format!("/{name}/{version}_meta.json"))
        .and_then(|t| serde_json::Deserializer::from_str(&t).into_iter().next()?.ok());
    let exports = meta.iter().flat_map(|m| m["exports"].as_object().into_iter().flatten().filter_map(|(_, v)| v.as_str()));
    let entries: Vec<String> = exports.map(str::to_string).chain(["./mod.ts".to_string()]).collect();
    entries.iter().find_map(|entry| {
        let source = read(format!("/{name}/{version}/{}", entry.trim_start_matches("./")))?;
        source.lines().take(5).filter(|l| l.trim_start().starts_with("//")).find_map(|l| {
            l.to_lowercase().contains("license").then(|| spdx_from_name(l)).flatten().map(str::to_string)
        })
    })
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let mut deps = Vec::new();
    let mut own = None;
    let mut inside = false;
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        let strings = quoted(t);
        if t.starts_with('}') {
            inside = false;
        } else if strings.first() == Some(&"imports") {
            inside = true;
        } else if inside && let Some(spec) = strings.get(1) {
            deps.extend(package_of(spec).and_then(|name| Mention::find(raw, line, &name)));
        } else if !inside && strings.first() == Some(&"license") {
            own = Mention::find(raw, line, "license");
        }
    }
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    super::npm::import(line)
}

#[std::prelude::rust_2024::test]
fn finds_deno_packages() {
    let config = "{\n  \"license\": \"MIT\",\n  \"imports\": {\n    \"@std/path\": \"jsr:@std/path@^1.0.8\",\n    \"chalk\": \"npm:chalk@5\"\n  }\n}\n";
    let (deps, own) = mentions(config);
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("@std/path", 3), ("chalk", 4)]);
    assert_eq!(own.map(|m| m.line), Some(1));
}

/// The cache layout as Deno 2.9 writes it: files named by the SHA-256 of the URL path.
#[std::prelude::rust_2024::test]
fn reads_jsr_license_from_the_deno_cache() {
    use sha2::{Digest, Sha256};
    let hashed = |path: &str| {
        let hash: String = Sha256::digest(path.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        format!("remote/https/jsr.io/{hash}")
    };
    let meta = hashed("/@std/path/1.1.6_meta.json");
    let entry = hashed("/@std/path/1.1.6/mod.ts");
    let cache = super::scratch("deno-cache", &[
        (&meta, "{\"exports\":{\".\":\"./mod.ts\"}}\n// x-deno-header"),
        (&entry, "// Copyright 2018-2026 the Deno authors. MIT license.\nexport * from \"./join.ts\";\n"),
    ]);
    assert_eq!(jsr_license(&cache, "@std/path", "1.1.6").as_deref(), Some("MIT"));
    std::fs::remove_dir_all(&cache).unwrap();
}
