//! CocoaPods and Carthage, the package managers Swift had before SwiftPM.

use std::path::Path;

use super::{Dep, Mention, Report, license_file, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    if manifest.file_name()? == "Cartfile" {
        let resolved = std::fs::read_to_string(dir.join("Cartfile.resolved")).unwrap_or_default();
        let deps = resolved
            .lines()
            .filter_map(|l| {
                let q = quoted(l);
                let name = repo_name(q.first()?);
                let license = license_file(&dir.join("Carthage/Checkouts").join(name));
                Some(Dep::new(name, q.get(1).copied().unwrap_or_default(), license))
            })
            .collect();
        return Some(Report::flat(license_file(dir), deps));
    }
    let lock = std::fs::read_to_string(dir.join("Podfile.lock")).unwrap_or_default();
    let (pods, requires, roots) = podfile_lock(&lock);
    let deps = pods
        .into_iter()
        .map(|(name, version)| {
            let license = license_file(&dir.join("Pods").join(&name));
            Dep::new(name, version, license)
        })
        .collect();
    let mut report = Report::flat(license_file(dir), deps);
    report.link(&requires, &roots);
    Some(report)
}

/// A subspec ("Firebase/Core") belongs to its pod.
fn pod(spec: &str) -> String {
    spec.split(['/', ' ']).next().unwrap_or(spec).trim_matches('"').to_string()
}

type Locked = (Vec<(String, String)>, Vec<(String, Vec<String>)>, Vec<String>);

/// PODS lists each pod at "  - " with what it needs at "    - "; DEPENDENCIES names the Podfile's own.
fn podfile_lock(lock: &str) -> Locked {
    let (mut pods, mut requires, mut roots): Locked = Default::default();
    let mut section = "";
    for line in lock.lines() {
        if !line.starts_with(' ') {
            section = line.trim_end_matches(':');
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let Some(spec) = line.trim().strip_prefix("- ") else { continue };
        let spec = spec.trim_end_matches(':');
        match (section, indent) {
            ("PODS", 2) => {
                let name = pod(spec);
                let version = spec.split_once(" (").map_or("", |(_, v)| v.trim_end_matches(')')).to_string();
                if !pods.iter().any(|(n, _)| *n == name) {
                    pods.push((name.clone(), version));
                }
                requires.push((name, Vec::new()));
            }
            ("PODS", 4) => requires.last_mut().into_iter().for_each(|r| r.1.push(pod(spec))),
            ("DEPENDENCIES", 2) => roots.push(pod(spec)),
            _ => {}
        }
    }
    (pods, requires, roots)
}

fn repo_name(source: &str) -> &str {
    let last = source.trim_end_matches('/').rsplit('/').next().unwrap_or(source);
    last.strip_suffix(".git").unwrap_or(last)
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let deps = text
        .lines()
        .enumerate()
        .filter_map(|(line, raw)| {
            let t = raw.trim_start();
            let rest = t.strip_prefix("pod ").or_else(|| t.strip_prefix("github ")).or_else(|| t.strip_prefix("git "))?;
            let first = *quoted(rest).first()?;
            let name = if t.starts_with("pod ") { pod(first) } else { repo_name(first).to_string() };
            Mention::find(raw, line, &name)
        })
        .collect();
    (deps, None)
}

pub fn import(line: &str) -> Option<String> {
    super::swift::import(line)
}

#[std::prelude::rust_2024::test]
fn reads_pods_and_carthage() {
    let (deps, _) = mentions("target 'App' do\n  pod 'Alamofire', '~> 5.8'\n  pod 'Firebase/Analytics'\nend\n");
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("Alamofire", 1), ("Firebase", 2)]);
    let (deps, _) = mentions("github \"ReactiveX/RxSwift\" ~> 6.0\n");
    assert_eq!(deps.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(), ["RxSwift"]);
    let lock = "PODS:\n  - Alamofire (5.8.1)\n  - Firebase/Analytics (10.0.0):\n    - FirebaseAnalytics (~> 10.0.0)\n  - FirebaseAnalytics (10.0.0)\n\nDEPENDENCIES:\n  - Alamofire (~> 5.8)\n  - Firebase/Analytics\n";
    let (pods, requires, roots) = podfile_lock(lock);
    assert_eq!(pods.iter().map(|(n, v)| format!("{n} {v}")).collect::<Vec<_>>(), ["Alamofire 5.8.1", "Firebase 10.0.0", "FirebaseAnalytics 10.0.0"]);
    assert_eq!(requires[1], ("Firebase".to_string(), vec!["FirebaseAnalytics".to_string()]));
    assert_eq!(roots, ["Alamofire", "Firebase"]);
}
