mod cargo;
mod cocoapods;
mod conan;
mod conda;
mod deno;
mod detect;
mod dotnet;
mod go;
mod java;
mod lua;
mod npm;
mod php;
mod python;
mod ruby;
mod spdx;
pub(crate) mod swift;
mod text;
mod vcpkg;

use std::{
    ops::Range,
    path::{Path, PathBuf},
};

use detect::{detect, license_file, normalize, spdx_from_name};
pub use spdx::{Family, Verdict, family, verdict};
#[cfg(test)]
use text::scratch;
use text::{jsonc, quoted};

#[derive(Default)]
pub struct Report {
    pub own: Option<String>,
    pub deps: Vec<Dep>,
    /// (dependent, dependency) as indices into `deps`, where the lockfile tells.
    pub edges: Vec<(usize, usize)>,
    /// What the package itself depends on; empty when only a flat list is known.
    pub roots: Vec<usize>,
}

impl Report {
    fn flat(own: Option<String>, deps: Vec<Dep>) -> Self {
        Self { own, deps, ..Self::default() }
    }

    fn link(&mut self, requires: &[(String, Vec<String>)], roots: &[String]) {
        let mut by_name: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (ix, dep) in self.deps.iter().enumerate() {
            by_name.entry(norm(&dep.name)).or_insert(ix);
        }
        let find = |name: &str| by_name.get(&norm(name)).copied();
        for (from, names) in requires {
            let Some(from) = find(from) else { continue };
            self.edges.extend(names.iter().filter_map(|n| find(n)).map(|to| (from, to)));
        }
        self.roots = roots.iter().filter_map(|n| find(n)).collect();
    }
}

pub struct Dep {
    pub name: String,
    pub version: String,
    pub license: Option<String>,
    pub shipped: bool,
    /// Names code imports it by, when they differ from `name`: modules, namespaces, group ids.
    pub aliases: Vec<String>,
}

impl Dep {
    pub fn new(name: impl Into<String>, version: impl Into<String>, license: Option<String>) -> Self {
        Self { name: name.into(), version: version.into(), license, shipped: true, aliases: Vec::new() }
    }
}

pub struct Line {
    pub summary: String,
    pub label: String,
    pub verdict: Verdict,
}

impl Line {
    pub fn markdown(&self) -> String {
        match &self.verdict {
            Verdict::Ok => self.summary.clone(),
            Verdict::Caution(why) | Verdict::Unknown(why) => format!("{}\n\n⚠️ {why}", self.summary),
            Verdict::Conflict(why) => format!("{}\n\n⛔ {why}", self.summary),
        }
    }
}

pub type Lines = std::rc::Rc<std::cell::RefCell<std::collections::HashMap<u32, Line>>>;

#[derive(Debug, PartialEq)]
pub struct Mention {
    pub name: String,
    pub line: u32,
    pub cols: Range<u32>,
}

impl Mention {
    fn find(raw: &str, line: usize, name: &str) -> Option<Self> {
        let start = raw.find(name)? as u32;
        Some(Self { name: name.to_string(), line: line as u32, cols: start..start + name.len() as u32 })
    }

    fn named(raw: &str, line: usize, at: &str, name: &str) -> Option<Self> {
        Some(Self { name: name.to_string(), ..Self::find(raw, line, at)? })
    }
}

struct Ecosystem {
    is_manifest: fn(&str) -> bool,
    locks: &'static [&'static str],
    sources: &'static [&'static str],
    report: fn(&Path) -> Option<Report>,
    mentions: fn(&str) -> (Vec<Mention>, Option<Mention>),
    import: fn(&str) -> Option<String>,
}

const ECOSYSTEMS: &[Ecosystem] = &[
    Ecosystem {
        is_manifest: |n| n == "Cargo.toml",
        locks: &["Cargo.lock"],
        sources: &["rs"],
        report: cargo::report,
        mentions: cargo::mentions,
        import: cargo::import,
    },
    Ecosystem {
        is_manifest: |n| n == "package.json",
        locks: &["package-lock.json", "yarn.lock", "pnpm-lock.yaml", "bun.lock"],
        sources: &["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts", "vue", "svelte"],
        report: npm::report,
        mentions: npm::mentions,
        import: npm::import,
    },
    Ecosystem {
        is_manifest: |n| matches!(n, "deno.json" | "deno.jsonc"),
        locks: &["deno.lock"],
        sources: &["js", "jsx", "mjs", "ts", "tsx", "mts"],
        report: deno::report,
        mentions: deno::mentions,
        import: deno::import,
    },
    Ecosystem {
        is_manifest: |n| matches!(n, "pyproject.toml" | "requirements.txt" | "Pipfile"),
        locks: &["uv.lock", "poetry.lock", "Pipfile.lock"],
        sources: &["py"],
        report: python::report,
        mentions: python::mentions,
        import: python::import,
    },
    Ecosystem {
        is_manifest: |n| matches!(n, "pixi.toml" | "environment.yml" | "environment.yaml"),
        locks: &["pixi.lock"],
        sources: &["py"],
        report: conda::report,
        mentions: conda::mentions,
        import: python::import,
    },
    Ecosystem {
        is_manifest: |n| n == "go.mod",
        locks: &["go.sum"],
        sources: &["go"],
        report: go::report,
        mentions: go::mentions,
        import: go::import,
    },
    Ecosystem {
        is_manifest: |n| n == "Gemfile",
        locks: &["Gemfile.lock"],
        sources: &["rb"],
        report: ruby::report,
        mentions: ruby::mentions,
        import: ruby::import,
    },
    Ecosystem {
        is_manifest: |n| n == "composer.json",
        locks: &["composer.lock"],
        sources: &["php"],
        report: php::report,
        mentions: php::mentions,
        import: php::import,
    },
    Ecosystem {
        is_manifest: |n| n.ends_with(".csproj") || n.ends_with(".fsproj") || n == "Directory.Packages.props",
        locks: &["packages.lock.json", "project.assets.json"],
        sources: &["cs"],
        report: dotnet::report,
        mentions: dotnet::mentions,
        import: dotnet::import,
    },
    Ecosystem {
        is_manifest: |n| matches!(n, "pom.xml" | "build.gradle" | "build.gradle.kts"),
        locks: &["gradle.lockfile"],
        sources: &["java", "kt", "kts"],
        report: java::report,
        mentions: java::mentions,
        import: java::import,
    },
    Ecosystem {
        is_manifest: |n| n == "Package.swift",
        locks: &["Package.resolved"],
        sources: &["swift"],
        report: swift::report,
        mentions: swift::mentions,
        import: swift::import,
    },
    Ecosystem {
        is_manifest: |n| matches!(n, "Podfile" | "Cartfile"),
        locks: &["Podfile.lock", "Cartfile.resolved"],
        sources: &["swift"],
        report: cocoapods::report,
        mentions: cocoapods::mentions,
        import: cocoapods::import,
    },
    Ecosystem {
        is_manifest: |n| n.ends_with(".rockspec"),
        locks: &[],
        sources: &["lua"],
        report: lua::report,
        mentions: lua::mentions,
        import: lua::import,
    },
    Ecosystem {
        is_manifest: |n| n == "vcpkg.json",
        locks: &[],
        sources: &["c", "h", "cc", "cpp", "cxx", "hpp", "hh"],
        report: vcpkg::report,
        mentions: vcpkg::mentions,
        import: vcpkg::import,
    },
    Ecosystem {
        is_manifest: |n| matches!(n, "conanfile.txt" | "conanfile.py"),
        locks: &["conan.lock"],
        sources: &["c", "h", "cc", "cpp", "cxx", "hpp", "hh"],
        report: conan::report,
        mentions: conan::mentions,
        import: conan::import,
    },
];

fn file_name(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or_default()
}

fn by_manifest(path: &Path) -> Option<&'static Ecosystem> {
    ECOSYSTEMS.iter().find(|e| (e.is_manifest)(file_name(path)))
}

pub fn is_manifest(path: &Path) -> bool {
    by_manifest(path).is_some()
}

pub fn affects(path: &Path) -> bool {
    let name = file_name(path);
    ECOSYSTEMS.iter().any(|e| (e.is_manifest)(name) || e.locks.contains(&name))
}

/// A language can have several package managers (npm and Deno, pip and conda); the nearest manifest wins.
pub fn manifest_for(file: &Path, root: &Path) -> Option<PathBuf> {
    if is_manifest(file) {
        return Some(file.to_path_buf());
    }
    let ext = file.extension()?.to_str()?;
    let ecos: Vec<&Ecosystem> = ECOSYSTEMS.iter().filter(|e| e.sources.contains(&ext)).collect();
    if ecos.is_empty() {
        return None;
    }
    for dir in file.ancestors().skip(1).take_while(|d| d.starts_with(root)) {
        let mut names: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).collect();
        names.sort();
        // Within one folder, the order of ECOSYSTEMS decides.
        let found = ecos.iter().find_map(|eco| names.iter().find(|p| (eco.is_manifest)(file_name(p))));
        if let Some(found) = found {
            return Some(found.clone());
        }
    }
    None
}

pub fn report(manifest: &Path) -> Option<Report> {
    (by_manifest(manifest)?.report)(manifest)
}

/// The second value is where a manifest states its own license.
pub fn mentions(path: &Path, manifest: &Path, text: &str) -> (Vec<Mention>, Option<Mention>) {
    if let Some(eco) = by_manifest(path) {
        return (eco.mentions)(text);
    }
    let Some(eco) = by_manifest(manifest) else {
        return (Vec::new(), None);
    };
    let imports = text
        .lines()
        .enumerate()
        .filter_map(|(line, raw)| Mention::find(raw, line, &(eco.import)(raw)?))
        .collect();
    (imports, None)
}

fn norm(name: &str) -> String {
    name.to_lowercase().replace(['_', '.'], "-")
}

pub fn find<'a>(deps: &'a [Dep], name: &str) -> Option<&'a Dep> {
    let wanted = norm(name);
    let lower = name.to_lowercase();
    let mut best: Option<(&Dep, usize, bool)> = None;
    for dep in deps {
        for key in std::iter::once(&dep.name).chain(&dep.aliases) {
            let exact = norm(key) == wanted;
            let key_lower = key.to_lowercase();
            let inside = lower
                .strip_prefix(&key_lower)
                .is_some_and(|rest| rest.starts_with(['/', '\\', '.', ':']));
            // The longest match is the most specific module; a shipped copy beats a dev-only one.
            let better = best.is_none_or(|(_, len, shipped)| (key.len(), dep.shipped) > (len, shipped));
            if (exact || inside) && better {
                best = Some((dep, key.len(), dep.shipped));
            }
        }
    }
    best.map(|(dep, _, _)| dep)
}

/// `PROBE=path/to/manifest cargo test probe -- --ignored --nocapture` prints what a real project yields.
#[std::prelude::rust_2024::test]
#[ignore = "reads a real project"]
fn probe() {
    let manifest = PathBuf::from(std::env::var("PROBE").expect("PROBE"));
    let report = report(&manifest).expect("report");
    println!("own: {:?} · {} deps · {} edges · roots {:?}", report.own, report.deps.len(), report.edges.len(),
        report.roots.iter().map(|&r| &report.deps[r].name).collect::<Vec<_>>());
    for d in &report.deps {
        println!("{} {} {:?} shipped={} aliases={:?}", d.name, d.version, d.license, d.shipped, d.aliases);
    }
}

#[std::prelude::rust_2024::test]
fn matches_imports_to_packages() {
    let dep = |name: &str, aliases: &[&str]| Dep { aliases: aliases.iter().map(|a| a.to_string()).collect(), ..Dep::new(name, "1", None) };
    let deps = [
        dep("serde-json", &[]),
        dep("github.com/a/b", &[]),
        dep("github.com/a/b/v2", &[]),
        dep("guzzlehttp/guzzle", &["GuzzleHttp"]),
        dep("PyYAML", &["yaml"]),
    ];
    let name = |n: &str| find(&deps, n).map(|d| d.name.as_str());
    assert_eq!(name("serde_json"), Some("serde-json"));
    assert_eq!(name("github.com/a/b/v2/pkg"), Some("github.com/a/b/v2"));
    assert_eq!(name("github.com/a/b/pkg"), Some("github.com/a/b"));
    assert_eq!(name("GuzzleHttp\\Client"), Some("guzzlehttp/guzzle"));
    assert_eq!(name("yaml"), Some("PyYAML"));
    assert_eq!(name("github.com/a/bc"), None);
}
