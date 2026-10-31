#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Ok,
    Caution(String),
    Unknown(String),
    Conflict(String),
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Permissive,
    WeakCopyleft,
    Gpl2Only,
    Gpl2Plus,
    Gpl3,
    Agpl3,
    Unknown,
}

pub(super) fn kind(id: &str) -> Kind {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Family {
    Permissive,
    WeakCopyleft,
    Copyleft,
    Unknown,
}

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

/// Old crates wrote "MIT/Apache-2.0".
pub(super) fn parse(expr: &str) -> Vec<Vec<String>> {
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
