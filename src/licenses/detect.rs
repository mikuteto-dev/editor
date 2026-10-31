use std::path::{Path, PathBuf};

use super::{
    file_name,
    spdx::{Kind, kind, parse},
};

pub(super) fn normalize(raw: &str) -> Option<String> {
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
pub(super) fn spdx_from_name(name: &str) -> Option<&'static str> {
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

pub(super) fn detect(text: &str) -> Option<&'static str> {
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

pub(super) fn license_file(dir: &Path) -> Option<String> {
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
