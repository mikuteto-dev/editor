use std::path::Path;

use serde_json::Value;

use super::{Dep, Mention, Report, license_file, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let resolved: Value =
        std::fs::read_to_string(dir.join("Package.resolved")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    // Version 1 nests the pins and names the URL differently.
    let pins = resolved["pins"].as_array().or(resolved["object"]["pins"].as_array()).cloned().unwrap_or_default();
    let deps = pins
        .iter()
        .filter_map(|pin| {
            let url = pin["location"].as_str().or(pin["repositoryURL"].as_str())?;
            let name = repo_name(url);
            let version = pin["state"]["version"].as_str().or(pin["state"]["revision"].as_str()).unwrap_or_default();
            // SwiftPM checks each package out under .build; the LICENSE there tells.
            let checkout = dir.join(".build/checkouts").join(name);
            let license = license_file(&checkout);
            // `import` names a module, which is a target: swift-collections provides Collections.
            let targets = std::fs::read_to_string(checkout.join("Package.swift")).map(|t| targets(&t)).unwrap_or_default();
            let identity = pin["identity"].as_str().map(str::to_string);
            Some(Dep { aliases: identity.into_iter().chain(targets).collect(), ..Dep::new(name, version, license) })
        })
        .collect();
    Some(Report::flat(license_file(dir), deps))
}

/// Names of the targets and products a Package.swift declares.
fn targets(manifest: &str) -> Vec<String> {
    let mut names: Vec<String> = [".target(", ".library(", ".executableTarget("]
        .iter()
        .flat_map(|kind| manifest.split(kind).skip(1))
        .filter_map(|rest| quoted(rest.split("name:").nth(1)?).first().map(|s| s.to_string()))
        .collect();
    names.sort();
    names.dedup();
    names
}

fn repo_name(url: &str) -> &str {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url);
    last.strip_suffix(".git").unwrap_or(last)
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let deps = text
        .lines()
        .enumerate()
        .filter_map(|(line, raw)| {
            let rest = raw.split(".package(").nth(1)?;
            let url = quoted(rest).into_iter().find(|q| q.contains('/'))?;
            Mention::find(raw, line, repo_name(url))
        })
        .collect();
    (deps, None)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start().trim_start_matches("@testable ").trim_start_matches("@_exported ");
    let rest = t.strip_prefix("import ")?;
    let words: Vec<&str> = rest.split_whitespace().collect();
    // `import struct Module.Type` names the module second.
    let module = if words.len() > 1 { words[1] } else { *words.first()? };
    Some(module.split('.').next()?.to_string())
}

#[std::prelude::rust_2024::test]
fn finds_swift_packages() {
    let manifest = "let package = Package(\n  dependencies: [\n    .package(url: \"https://github.com/Alamofire/Alamofire.git\", from: \"5.8.0\"),\n  ]\n)\n";
    let (deps, _) = mentions(manifest);
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("Alamofire", 2)]);
    assert_eq!(import("import Alamofire").as_deref(), Some("Alamofire"));
    assert_eq!(import("@testable import struct Foo.Bar").as_deref(), Some("Foo"));
    let collections = "let package = Package(\n  name: \"swift-collections\",\n  products: [\n    .library(name: \"Collections\", targets: [\"Collections\"]),\n  ],\n  targets: [\n    .target(name: \"DequeModule\"),\n  ]\n)\n";
    assert_eq!(targets(collections), ["Collections", "DequeModule"]);
    let resolved = r#"{"pins": [{"identity": "alamofire", "location": "https://github.com/Alamofire/Alamofire.git", "state": {"version": "5.8.1"}}], "version": 2}"#;
    let dir = super::scratch("swift", &[
        ("Package.swift", manifest),
        ("Package.resolved", resolved),
        (".build/checkouts/Alamofire/LICENSE", "Permission is hereby granted, free of charge, to any person"),
    ]);
    let report = report(&dir.join("Package.swift")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let dep = super::find(&report.deps, "Alamofire").unwrap();
    assert_eq!((dep.version.as_str(), dep.license.as_deref()), ("5.8.1", Some("MIT")));
}
