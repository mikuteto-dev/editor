use std::path::{Path, PathBuf};

use super::{Dep, Mention, Report, license_file, normalize, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let lock = std::fs::read_to_string(dir.join("Gemfile.lock")).unwrap_or_default();
    let spec_dirs = spec_dirs(dir);
    let (specs, requires, roots) = locked(&lock);
    let deps = specs
        .into_iter()
        .map(|(name, version)| {
            let spec = spec_dirs.iter().map(|d| d.join(format!("{name}-{version}.gemspec"))).find(|p| p.is_file());
            let license = spec.as_deref().and_then(gemspec_license);
            // What `require` loads is whatever the gem puts in lib/: "active_support" for activesupport.
            let lib = spec.as_deref().and_then(|s| Some(s.parent()?.parent()?.join("gems").join(format!("{name}-{version}/lib"))));
            let aliases = lib
                .iter()
                .flat_map(|lib| std::fs::read_dir(lib).into_iter().flatten().flatten())
                .map(|e| e.file_name().to_string_lossy().trim_end_matches(".rb").to_string())
                .collect();
            Dep { aliases, ..Dep::new(name, version, license) }
        })
        .collect();
    let own = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "gemspec"))
        .and_then(|p| gemspec_license(&p))
        .or_else(|| license_file(dir));
    let mut report = Report::flat(own, deps);
    report.link(&requires, &roots);
    Some(report)
}

/// Where installed gems keep their specifications: the bundle, then the usual Ruby installs.
fn spec_dirs(dir: &Path) -> Vec<PathBuf> {
    let home = std::env::home_dir().unwrap_or_default();
    let mut roots = vec![dir.join("vendor/bundle/ruby")];
    roots.extend(std::env::var_os("GEM_HOME").map(PathBuf::from));
    roots.extend([".gem/ruby", ".local/share/gem/ruby"].map(|p| home.join(p)));
    roots.push("/opt/homebrew/lib/ruby/gems".into());
    roots.push("/usr/local/lib/ruby/gems".into());
    let rubies = std::fs::read_dir(home.join(".rbenv/versions")).into_iter().flatten().flatten();
    roots.extend(rubies.map(|v| v.path().join("lib/ruby/gems")));
    roots
        .iter()
        .flat_map(|root| {
            // Each root holds one directory per Ruby version, or is one already.
            let versions = std::fs::read_dir(root).into_iter().flatten().flatten().map(|e| e.path());
            std::iter::once(root.clone()).chain(versions).map(|p| p.join("specifications")).collect::<Vec<_>>()
        })
        .filter(|p| p.is_dir())
        .collect()
}

fn gemspec_license(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.lines().find(|l| l.contains(".licenses =") || l.contains(".license ="))?;
    let ids: Vec<&str> = quoted(line).into_iter().filter(|q| !q.is_empty()).collect();
    normalize(&ids.join(" OR "))
}

/// Gemfile.lock: `name (version)` specs at four spaces with what they require at six,
/// and the Gemfile's own gems under DEPENDENCIES.
type Locked = (Vec<(String, String)>, Vec<(String, Vec<String>)>, Vec<String>);

fn locked(lock: &str) -> Locked {
    let (mut specs, mut requires, mut roots) = (Vec::new(), Vec::<(String, Vec<String>)>::new(), Vec::new());
    let mut section = "";
    for line in lock.lines() {
        let indent = line.len() - line.trim_start().len();
        let word = line.trim().split([' ', '!']).next().unwrap_or_default().to_string();
        match indent {
            0 => section = if line == "DEPENDENCIES" { "roots" } else { "" },
            2 if section == "roots" => roots.push(word),
            4 => {
                if let Some((name, version)) = line.trim().split_once(" (") {
                    specs.push((name.to_string(), version.trim_end_matches(')').to_string()));
                    requires.push((name.to_string(), Vec::new()));
                }
            }
            6 => requires.last_mut().into_iter().for_each(|r| r.1.push(word.clone())),
            _ => {}
        }
    }
    (specs, requires, roots)
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let deps = text
        .lines()
        .enumerate()
        .filter_map(|(line, raw)| {
            let rest = raw.trim_start().strip_prefix("gem ")?;
            Mention::find(raw, line, quoted(rest).first()?)
        })
        .collect();
    (deps, None)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix("require ").or_else(|| t.strip_prefix("require("))?;
    Some(quoted(rest).first()?.split('/').next()?.to_string())
}

#[std::prelude::rust_2024::test]
fn finds_gems() {
    let (deps, _) = mentions("source \"https://rubygems.org\"\n\ngem \"rails\", \"~> 7\"\ngem 'pg'\n");
    let names: Vec<_> = deps.iter().map(|m| (m.name.as_str(), m.line, m.cols.clone())).collect();
    assert_eq!(names, [("rails", 2, 5..10), ("pg", 3, 5..7)]);
    assert_eq!(import("require \"json\"").as_deref(), Some("json"));
    assert_eq!(import("require 'rails/all'").as_deref(), Some("rails"));
    assert_eq!(import("require_relative 'x'"), None);
    let lock = "GEM\n  remote: https://rubygems.org/\n  specs:\n    rack (3.0.8)\n    rails (7.1.0)\n      rack (>= 2)\n\nDEPENDENCIES\n  rails (~> 7)\n";
    let (specs, requires, roots) = locked(lock);
    assert_eq!(specs, [("rack".to_string(), "3.0.8".to_string()), ("rails".to_string(), "7.1.0".to_string())]);
    assert_eq!(requires[1], ("rails".to_string(), vec!["rack".to_string()]));
    assert_eq!(roots, ["rails"]);
    let dir = super::scratch("gem", &[("x.gemspec", "  s.licenses = [\"MIT\".freeze]\n")]);
    assert_eq!(gemspec_license(&dir.join("x.gemspec")).as_deref(), Some("MIT"));
    std::fs::remove_dir_all(&dir).unwrap();
}
