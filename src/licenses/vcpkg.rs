use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Dep, Mention, Report, detect, license_file, normalize, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let json: Value = serde_json::from_str(&std::fs::read_to_string(manifest).ok()?).ok()?;
    let own = json["license"].as_str().and_then(normalize).or_else(|| license_file(dir));
    let root = std::env::var_os("VCPKG_ROOT").map(PathBuf::from);
    let deps = json["dependencies"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d.as_str().or(d["name"].as_str()))
        .map(|name| {
            // The port's manifest states an SPDX license; an installed port keeps its copyright file.
            let port = root.as_ref().and_then(|r| read_json(&r.join("ports").join(name).join("vcpkg.json")));
            let license = port.as_ref().and_then(|p| p["license"].as_str()).and_then(normalize).or_else(|| installed_license(dir, name));
            let version = port.as_ref().and_then(|p| p["version"].as_str().or(p["version-string"].as_str())).unwrap_or_default();
            Dep { aliases: headers(dir, name), ..Dep::new(name, version, license) }
        })
        .collect();
    Some(Report::flat(own, deps))
}

/// The include paths a port installed, two levels deep ("boost/asio", "zlib"), from vcpkg's file lists.
fn headers(dir: &Path, port: &str) -> Vec<String> {
    let info = dir.join("vcpkg_installed/vcpkg/info");
    let prefix = format!("{port}_");
    let lists = std::fs::read_dir(info).into_iter().flatten().flatten().filter(|e| e.file_name().to_string_lossy().starts_with(&prefix));
    let mut found: Vec<String> = lists
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .flat_map(|text| text.lines().filter_map(|l| l.split_once("/include/").map(|(_, h)| h.to_string())).collect::<Vec<_>>())
        .filter(|h| !h.ends_with('/'))
        .map(|h| {
            let parts: Vec<&str> = h.split('/').collect();
            let take = if parts.len() > 2 { 2 } else { parts.len() };
            let joined = parts[..take].join("/");
            // A header right under include/ is named without its extension, as the import is.
            joined.rsplit_once('.').filter(|_| take == parts.len()).map_or(joined.clone(), |(stem, _)| stem.to_string())
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn installed_license(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_dir(dir.join("vcpkg_installed"))
        .ok()?
        .flatten()
        .find_map(|triplet| detect(&std::fs::read_to_string(triplet.path().join("share").join(name).join("copyright")).ok()?))
        .map(str::to_string)
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let mut deps = Vec::new();
    let mut own = None;
    let mut inside = false;
    for (line, raw) in text.lines().enumerate() {
        let t = raw.trim();
        let strings = quoted(t);
        if strings.first() == Some(&"dependencies") {
            inside = true;
            continue;
        }
        if inside && t.starts_with(']') {
            inside = false;
        } else if inside && strings.first() == Some(&"name") {
            deps.extend(strings.get(1).and_then(|n| Mention::find(raw, line, n)));
        } else if inside && t.starts_with('"') && !t.contains(':') {
            deps.extend(strings.first().and_then(|n| Mention::find(raw, line, n)));
        } else if !inside && strings.first() == Some(&"license") {
            own = Mention::find(raw, line, "license");
        }
    }
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    let rest = line.trim_start().strip_prefix("#include")?.trim_start().strip_prefix('<')?;
    let header = rest.split('>').next()?;
    Some(header.rsplit_once('.').map_or(header, |(stem, _)| stem).to_string())
}

#[std::prelude::rust_2024::test]
fn finds_ports() {
    let json = "{\n  \"name\": \"app\",\n  \"license\": \"MIT\",\n  \"dependencies\": [\n    \"fmt\",\n    {\n      \"name\": \"zlib\",\n      \"default-features\": false\n    }\n  ]\n}\n";
    let (deps, own) = mentions(json);
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("fmt", 4), ("zlib", 6)]);
    assert_eq!(own.map(|m| m.line), Some(2));
    assert_eq!(import("#include <fmt/core.h>").as_deref(), Some("fmt/core"));
    assert_eq!(import("#include <zlib.h>").as_deref(), Some("zlib"));
    assert_eq!(import("#include \"local.h\""), None);
    let list = "x64-osx/\nx64-osx/include/boost/asio.hpp\nx64-osx/include/boost/asio/io_context.hpp\nx64-osx/share/boost-asio/copyright\n";
    let dir = super::scratch("vcpkg", &[("vcpkg_installed/vcpkg/info/boost-asio_1.84.0_x64-osx.list", list)]);
    let aliases = headers(&dir, "boost-asio");
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(aliases, ["boost/asio"]);
    let deps = [super::Dep { aliases, ..super::Dep::new("boost-asio", "1.84.0", None) }];
    assert_eq!(super::find(&deps, "boost/asio/io_context").map(|d| d.name.as_str()), Some("boost-asio"));
    assert_eq!(super::find(&deps, "boost/asio").map(|d| d.name.as_str()), Some("boost-asio"));
}
