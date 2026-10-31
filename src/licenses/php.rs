use std::{collections::HashSet, path::Path};

use serde_json::Value;

use super::{Dep, Mention, Report, license_file, normalize, npm::json_mentions};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let composer: Value = serde_json::from_str(&std::fs::read_to_string(manifest).ok()?).ok()?;
    let own = license_of(&composer["license"]).or_else(|| license_file(dir));
    let installed: Value = std::fs::read_to_string(dir.join("vendor/composer/installed.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    // Composer 2 wraps the list and names the dev-only packages; Composer 1 is the bare list.
    let packages = installed["packages"].as_array().or(installed.as_array()).cloned().unwrap_or_default();
    let dev: HashSet<&str> =
        installed["dev-package-names"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    let deps = packages
        .iter()
        .filter_map(|p| {
            let name = p["name"].as_str()?;
            let autoload = ["psr-4", "psr-0"].iter().filter_map(|k| p["autoload"][k].as_object());
            let aliases = autoload.flatten().map(|(ns, _)| ns.trim_end_matches('\\').to_string()).filter(|ns| !ns.is_empty());
            Some(Dep {
                shipped: !dev.contains(name),
                aliases: aliases.collect(),
                ..Dep::new(name, p["version"].as_str().unwrap_or_default(), license_of(&p["license"]))
            })
        })
        .collect();
    let keys = |v: &Value| -> Vec<String> { v.as_object().into_iter().flatten().map(|(k, _)| k.clone()).collect() };
    let requires: Vec<(String, Vec<String>)> =
        packages.iter().filter_map(|p| Some((p["name"].as_str()?.to_string(), keys(&p["require"])))).collect();
    let roots = [keys(&composer["require"]), keys(&composer["require-dev"])].concat();
    let mut report = Report::flat(own, deps);
    report.link(&requires, &roots);
    Some(report)
}

fn license_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => normalize(s),
        Value::Array(list) => normalize(&list.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" OR ")),
        _ => None,
    }
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let (mut deps, own) = json_mentions(text, &["require", "require-dev"]);
    // The PHP version and extensions are platform requirements, not packages.
    deps.retain(|m| m.name != "php" && !m.name.starts_with("ext-"));
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start().strip_prefix("use ")?;
    let t = t.strip_prefix("function ").or_else(|| t.strip_prefix("const ")).unwrap_or(t);
    let path = t.split([';', ' ', '{']).next()?.trim_start_matches('\\').trim_end_matches('\\');
    path.contains('\\').then(|| path.to_string())
}

#[std::prelude::rust_2024::test]
fn finds_composer_packages() {
    let json = "{\n    \"license\": \"MIT\",\n    \"require\": {\n        \"php\": \">=8.1\",\n        \"guzzlehttp/guzzle\": \"^7\"\n    }\n}\n";
    let (deps, own) = mentions(json);
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("guzzlehttp/guzzle", 4)]);
    assert_eq!(own.map(|m| m.line), Some(1));
    assert_eq!(import("use GuzzleHttp\\Client;").as_deref(), Some("GuzzleHttp\\Client"));
    assert_eq!(import("use function Foo\\bar;").as_deref(), Some("Foo\\bar"));
    assert_eq!(import("use Trait1;"), None);
    let installed = r#"{"packages": [{"name": "guzzlehttp/guzzle", "version": "7.8.0", "license": ["MIT"], "autoload": {"psr-4": {"GuzzleHttp\\": "src/"}}}, {"name": "phpunit/phpunit", "version": "10", "license": ["BSD-3-Clause"]}], "dev-package-names": ["phpunit/phpunit"]}"#;
    let dir = super::scratch("php", &[("composer.json", "{\"license\": \"GPL-2.0-only\"}"), ("vendor/composer/installed.json", installed)]);
    let report = report(&dir.join("composer.json")).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let guzzle = super::find(&report.deps, "GuzzleHttp\\Client").unwrap();
    assert_eq!((guzzle.name.as_str(), guzzle.license.as_deref(), guzzle.shipped), ("guzzlehttp/guzzle", Some("MIT"), true));
    assert!(!super::find(&report.deps, "phpunit/phpunit").unwrap().shipped);
}
