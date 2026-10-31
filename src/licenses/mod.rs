//! Dependency licenses from what each package manager already keeps on disk, and whether
//! they fit the package's own license.

mod cargo;
mod cocoapods;
mod conan;
mod conda;
mod deno;
mod dotnet;
mod go;
mod java;
mod lua;
mod npm;
mod php;
mod python;
mod ruby;
pub(crate) mod swift;
mod vcpkg;

use std::{
    ops::Range,
    path::{Path, PathBuf},
};

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

    /// Links dependencies by name, for lockfiles that name what each package requires.
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
    /// Part of what the package ships; dev and build tools are not.
    pub shipped: bool,
    /// Names code imports it by, when they differ from `name`: modules, namespaces, group ids.
    pub aliases: Vec<String>,
}

impl Dep {
    pub fn new(name: impl Into<String>, version: impl Into<String>, license: Option<String>) -> Self {
        Self { name: name.into(), version: version.into(), license, shipped: true, aliases: Vec::new() }
    }
}

/// What a line of an open file says about a dependency, for hover and the status bar.
pub struct Line {
    pub summary: String,
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

/// A dependency named in a manifest or imported in source, by line and columns.
#[derive(Debug, PartialEq)]
pub struct Mention {
    pub name: String,
    pub line: u32,
    pub cols: Range<u32>,
}

impl Mention {
    /// `name` found in `raw` (line `line`); None if it is not there.
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
    /// Files whose change can change the resolved dependencies.
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

/// A change to this file can change some package's dependencies.
pub fn affects(path: &Path) -> bool {
    let name = file_name(path);
    ECOSYSTEMS.iter().any(|e| (e.is_manifest)(name) || e.locks.contains(&name))
}

/// The manifest that governs `file`: the file itself, or the nearest one above it inside `root`.
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

/// Dependencies named in `text`: manifest entries, or imports when `path` is source under `manifest`.
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

/// The dependency a manifest entry or import refers to; an import may name a module inside it.
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

/// A declared license as an SPDX expression where it can be recognized.
fn normalize(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("UNKNOWN") {
        return None;
    }
    // Some metadata carries the whole license text.
    if raw.len() > 200 || raw.contains('\n') {
        return detect(raw).map(str::to_string).or_else(|| Some(raw.lines().next()?.trim().to_string()));
    }
    let known = parse(raw).iter().flatten().all(|id| kind(id) != Kind::Unknown);
    if known {
        Some(raw.to_string())
    } else {
        Some(spdx_from_name(raw).map_or_else(|| raw.to_string(), str::to_string))
    }
}

/// A license name, URL or classifier as written by hand, e.g. "The Apache Software License, Version 2.0".
fn spdx_from_name(name: &str) -> Option<&'static str> {
    let n = name.to_lowercase();
    let has = |s: &str| n.contains(s);
    let v3 = has("3");
    Some(match () {
        _ if has("affero") || has("agpl") => "AGPL-3.0",
        _ if has("lesser") || has("lgpl") || has("library general") => {
            if v3 { "LGPL-3.0" } else { "LGPL-2.1" }
        }
        _ if has("gpl") || has("general public") => {
            if v3 { "GPL-3.0" } else { "GPL-2.0" }
        }
        _ if has("apache") => "Apache-2.0",
        _ if has("mozilla") || has("mpl") => "MPL-2.0",
        _ if has("eclipse") || has("epl") => {
            if has("2") { "EPL-2.0" } else { "EPL-1.0" }
        }
        _ if has("bsd") => {
            if has("2-clause") || has("2 clause") || has("simplified") || has("freebsd") { "BSD-2-Clause" } else { "BSD-3-Clause" }
        }
        _ if has("mit") || has("expat") => "MIT",
        _ if has("isc") => "ISC",
        _ if has("unlicense") || has("public domain") => "Unlicense",
        _ if has("cc0") => "CC0-1.0",
        _ if has("boost") || has("bsl") => "BSL-1.0",
        _ if has("zlib") => "Zlib",
        _ if has("cddl") => "CDDL-1.0",
        _ if has("python") || has("psf") => "PSF-2.0",
        _ => return None,
    })
}

/// The license of a LICENSE file, by the phrases each license's text contains.
fn detect(text: &str) -> Option<&'static str> {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let has = |s: &str| t.contains(s);
    Some(match () {
        _ if has("GNU AFFERO GENERAL PUBLIC LICENSE") => "AGPL-3.0",
        _ if has("GNU LESSER GENERAL PUBLIC LICENSE") => {
            if has("Version 3") { "LGPL-3.0" } else { "LGPL-2.1" }
        }
        _ if has("GNU GENERAL PUBLIC LICENSE") => {
            if has("Version 3") { "GPL-3.0" } else { "GPL-2.0" }
        }
        _ if has("Mozilla Public License") => "MPL-2.0",
        _ if has("Apache License") => "Apache-2.0",
        _ if has("Eclipse Public License") => {
            if has("v 2.0") || has("Version 2.0") { "EPL-2.0" } else { "EPL-1.0" }
        }
        _ if has("Boost Software License") => "BSL-1.0",
        _ if has("This is free and unencumbered software released into the public domain") => "Unlicense",
        _ if has("Permission to use, copy, modify, and/or distribute") || has("ISC License") => "ISC",
        _ if has("Permission is hereby granted, free of charge") => "MIT",
        _ if has("Redistribution and use in source and binary forms") => {
            if has("Neither the name") || has("endorse or promote") { "BSD-3-Clause" } else { "BSD-2-Clause" }
        }
        _ if has("This software is provided 'as-is'") => "Zlib",
        _ if has("CC0") => "CC0-1.0",
        _ => return None,
    })
}

/// The license of the LICENSE (or COPYING) file in `dir`.
fn license_file(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = file_name(p).to_ascii_uppercase();
            name.starts_with("LICENSE") || name.starts_with("LICENCE") || name.starts_with("COPYING")
        })
        .collect();
    files.sort();
    let found: Vec<&str> = files.iter().filter_map(|f| detect(&std::fs::read_to_string(f).ok()?)).collect();
    // LICENSE-MIT next to LICENSE-APACHE means the user may pick either.
    (!found.is_empty()).then(|| found.join(" OR "))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Ok,
    Caution(String),
    Unknown(String),
    Conflict(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Permissive,
    WeakCopyleft,
    Gpl2Only,
    Gpl2Plus,
    Gpl3,
    Agpl3,
    Unknown,
}

fn kind(id: &str) -> Kind {
    match id.trim_end_matches('+') {
        "GPL-2.0" | "LGPL-2.1" if id.ends_with('+') => if id.starts_with('L') { Kind::WeakCopyleft } else { Kind::Gpl2Plus },
        "MIT" | "MIT-0" | "Apache-2.0" | "BSD-2-Clause" | "BSD-3-Clause" | "ISC" | "Zlib" | "Unlicense" | "0BSD"
        | "CC0-1.0" | "BSL-1.0" | "Unicode-3.0" | "Unicode-DFS-2016" | "BlueOak-1.0.0" | "WTFPL" | "Python-2.0"
        | "PSF-2.0" | "X11" | "NCSA" | "CC-BY-4.0" | "OpenSSL" | "curl" | "Ruby" | "BSD-2-Clause-Patent" => {
            Kind::Permissive
        }
        "MPL-2.0" | "LGPL-2.0" | "LGPL-2.1" | "LGPL-2.1-only" | "LGPL-2.1-or-later" | "LGPL-3.0" | "LGPL-3.0-only"
        | "LGPL-3.0-or-later" | "EPL-1.0" | "EPL-2.0" | "CDDL-1.0" | "CDDL-1.1" => Kind::WeakCopyleft,
        "GPL-2.0" | "GPL-2.0-only" => Kind::Gpl2Only,
        "GPL-2.0-or-later" => Kind::Gpl2Plus,
        "GPL-3.0" | "GPL-3.0-only" | "GPL-3.0-or-later" => Kind::Gpl3,
        "AGPL-3.0" | "AGPL-3.0-only" | "AGPL-3.0-or-later" => Kind::Agpl3,
        _ => Kind::Unknown,
    }
}

/// How much a license asks of whoever ships it, for coloring at a glance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    Permissive,
    WeakCopyleft,
    Copyleft,
    Unknown,
}

/// The family of the least demanding option an expression offers.
pub fn family(license: Option<&str>) -> Family {
    let rank = |id: &String| match kind(id) {
        Kind::Permissive => Family::Permissive,
        Kind::WeakCopyleft => Family::WeakCopyleft,
        Kind::Gpl2Only | Kind::Gpl2Plus | Kind::Gpl3 | Kind::Agpl3 => Family::Copyleft,
        Kind::Unknown => Family::Unknown,
    };
    let options = license.map(parse).unwrap_or_default();
    options.iter().map(|all| all.iter().map(rank).max().unwrap_or(Family::Unknown)).min().unwrap_or(Family::Unknown)
}

/// An SPDX expression as alternatives (OR) of requirements (AND). Old crates wrote "MIT/Apache-2.0".
fn parse(expr: &str) -> Vec<Vec<String>> {
    let expr = expr.replace('/', " OR ").replace(['(', ')'], " ");
    // shortcut: parentheses are flattened, which is exact for the common "A OR B" and "(A OR B) AND C" with one group.
    let words: Vec<&str> = expr.split_whitespace().collect();
    let mut alternatives = vec![Vec::new()];
    let mut i = 0;
    while i < words.len() {
        match words[i] {
            "OR" | "or" => alternatives.push(Vec::new()),
            "AND" | "and" => {}
            // An exception only loosens the license before it.
            "WITH" | "with" => i += 1,
            id => alternatives.last_mut().unwrap().push(id.to_string()),
        }
        i += 1;
    }
    alternatives.retain(|a| !a.is_empty());
    alternatives
}

/// Whether a dependency under `dep` can ship inside a package under `own`.
pub fn verdict(own: Option<&str>, dep: Option<&str>) -> Verdict {
    let Some(dep) = dep.filter(|d| !d.trim().is_empty()) else {
        return Verdict::Unknown("No license declared".into());
    };
    let own_options = own.map(parse).unwrap_or_default();
    let own_options = if own_options.is_empty() { vec![vec![]] } else { own_options };
    // Every license the package offers must stay usable; within one, the dependency's best option counts.
    own_options
        .iter()
        .map(|own| {
            parse(dep)
                .iter()
                .map(|all| all.iter().map(|id| single(own, id)).max().unwrap_or(Verdict::Ok))
                .min()
                .unwrap_or_else(|| Verdict::Unknown(format!("Unrecognized license: {dep}")))
        })
        .max()
        .unwrap_or(Verdict::Ok)
}

fn single(own: &[String], id: &str) -> Verdict {
    let own_name = if own.is_empty() { "no license (all rights reserved)".to_string() } else { own.join(" AND ") };
    let own_kinds: Vec<Kind> = own.iter().map(|o| kind(o)).collect();
    let has = |k: Kind| own_kinds.contains(&k);
    let conflict = || Verdict::Conflict(format!("{id} requires the whole program to be under a compatible GPL; this package is {own_name}"));
    match kind(id) {
        Kind::Permissive if id == "Apache-2.0" && has(Kind::Gpl2Only) => {
            Verdict::Conflict(format!("Apache-2.0 is incompatible with {own_name}"))
        }
        Kind::Permissive | Kind::WeakCopyleft => Verdict::Ok,
        Kind::Unknown => Verdict::Unknown(format!("Unrecognized license: {id}")),
        Kind::Gpl2Only if has(Kind::Gpl2Only) || has(Kind::Gpl2Plus) => Verdict::Ok,
        Kind::Gpl2Plus if has(Kind::Gpl2Only) || has(Kind::Gpl2Plus) || has(Kind::Gpl3) || has(Kind::Agpl3) => Verdict::Ok,
        Kind::Gpl3 if has(Kind::Gpl2Plus) || has(Kind::Gpl3) || has(Kind::Agpl3) => Verdict::Ok,
        Kind::Agpl3 if has(Kind::Agpl3) => Verdict::Ok,
        Kind::Agpl3 if has(Kind::Gpl3) => Verdict::Caution(format!("{id} adds network-use terms to this {own_name} package")),
        _ => conflict(),
    }
}

/// JSON with comments and trailing commas, as bun.lock and deno.json allow.
fn jsonc(text: &str) -> Option<serde_json::Value> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                while chars.next_if(|&n| n != '\n').is_some() {}
            }
            ('/', Some('*')) => {
                chars.next();
                while let Some(n) = chars.next() {
                    if n == '*' && chars.next_if_eq(&'/').is_some() {
                        break;
                    }
                }
            }
            (']' | '}', _) => {
                let trimmed = out.trim_end().len();
                if out[..trimmed].ends_with(',') {
                    out.truncate(trimmed - 1);
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    serde_json::from_str(&out).ok()
}

/// Quoted strings in a line, in order.
fn quoted(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find(['"', '\'']) {
        let q = rest.as_bytes()[start] as char;
        let tail = &rest[start + 1..];
        let Some(end) = tail.find(q) else { break };
        out.push(&tail[..end]);
        rest = &tail[end + 1..];
    }
    out
}

/// Tests read package-manager layouts from a scratch directory.
#[cfg(test)]
fn scratch(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("editor-lic-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

#[std::prelude::rust_2024::test]
fn judges_license_fit() {
    use Verdict::*;
    let ok = |own: Option<&str>, dep| verdict(own, Some(dep)) == Ok;
    let conflict = |own: Option<&str>, dep| matches!(verdict(own, Some(dep)), Conflict(_));
    assert!(ok(Some("MIT"), "MIT OR Apache-2.0"));
    assert!(ok(Some("MIT"), "MIT/Apache-2.0"));
    assert!(ok(Some("MIT"), "MPL-2.0"));
    assert!(conflict(Some("MIT"), "GPL-3.0-only"));
    assert!(conflict(None, "GPL-2.0-or-later"));
    // A dual license lets the dependency pick its permissive side.
    assert!(ok(Some("MIT"), "GPL-3.0 OR MIT"));
    assert!(ok(Some("GPL-3.0-or-later"), "GPL-2.0-or-later"));
    assert!(conflict(Some("GPL-3.0-only"), "GPL-2.0-only"));
    assert!(conflict(Some("GPL-2.0-only"), "Apache-2.0"));
    assert!(ok(Some("GPL-3.0-or-later"), "Apache-2.0 WITH LLVM-exception"));
    // Every license a dual-licensed package offers must hold.
    assert!(conflict(Some("MIT OR GPL-3.0"), "GPL-3.0"));
    assert!(matches!(verdict(Some("GPL-3.0"), Some("AGPL-3.0-only")), Caution(_)));
    assert!(matches!(verdict(Some("MIT"), None), Unknown(_)));
    assert!(matches!(verdict(Some("MIT"), Some("Custom-1")), Unknown(_)));
}

#[std::prelude::rust_2024::test]
fn sorts_licenses_into_families() {
    assert_eq!(family(Some("MIT OR Apache-2.0")), Family::Permissive);
    assert_eq!(family(Some("GPL-3.0 OR MIT")), Family::Permissive);
    assert_eq!(family(Some("MPL-2.0")), Family::WeakCopyleft);
    assert_eq!(family(Some("Apache-2.0 AND GPL-2.0-only")), Family::Copyleft);
    assert_eq!(family(Some("Custom")), Family::Unknown);
    assert_eq!(family(None), Family::Unknown);
}

#[std::prelude::rust_2024::test]
fn recognizes_licenses_written_by_hand() {
    assert_eq!(normalize("The Apache Software License, Version 2.0").as_deref(), Some("Apache-2.0"));
    assert_eq!(normalize("http://www.opensource.org/licenses/mit-license.php").as_deref(), Some("MIT"));
    assert_eq!(normalize("GNU Lesser General Public License v3").as_deref(), Some("LGPL-3.0"));
    assert_eq!(normalize("MIT OR Apache-2.0").as_deref(), Some("MIT OR Apache-2.0"));
    assert_eq!(normalize("UNKNOWN"), None);
    assert_eq!(detect("MIT License\n\nPermission is hereby granted, free of charge, to any person"), Some("MIT"));
    assert_eq!(detect("GNU LESSER GENERAL PUBLIC LICENSE\n Version 2.1, February 1999"), Some("LGPL-2.1"));
    let bsd = "Redistribution and use in source and binary forms ... Neither the name of the copyright holder";
    assert_eq!(detect(bsd), Some("BSD-3-Clause"));
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
fn reads_jsonc() {
    let text = "{\n  // comment\n  \"a\": \"x // not a comment\", /* block */\n  \"b\": [1, 2,],\n}\n";
    assert_eq!(jsonc(text), Some(serde_json::json!({"a": "x // not a comment", "b": [1, 2]})));
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
