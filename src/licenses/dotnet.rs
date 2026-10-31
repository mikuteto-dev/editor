use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Dep, Mention, Report, license_file, normalize, quoted};

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let project = std::fs::read_to_string(manifest).ok()?;
    let own = tag(&project, "PackageLicenseExpression").and_then(normalize).or_else(|| license_file(dir));
    let cache = std::env::var_os("NUGET_PACKAGES")
        .map(PathBuf::from)
        .or_else(|| Some(std::env::home_dir()?.join(".nuget/packages")))?;
    // After a restore, the assets file lists every package, transitive ones included.
    let assets: Option<Value> =
        std::fs::read_to_string(dir.join("obj/project.assets.json")).ok().and_then(|t| serde_json::from_str(&t).ok());
    let packages: Vec<(String, String)> = match assets.as_ref().and_then(|a| a["libraries"].as_object()) {
        Some(libraries) => libraries
            .iter()
            .filter(|(_, l)| l["type"] == "package")
            .filter_map(|(key, _)| {
                let (id, version) = key.split_once('/')?;
                Some((id.to_string(), version.to_string()))
            })
            .collect(),
        None => references(&project).map(|(_, id, version)| (id.to_string(), version.to_string())).collect(),
    };
    let deps = packages
        .into_iter()
        .map(|(id, version)| {
            let license = nuspec_license(&cache.join(id.to_lowercase()).join(&version), &id);
            Dep::new(id, version, license)
        })
        .collect();
    Some(Report::flat(own, deps))
}

fn nuspec_license(dir: &Path, id: &str) -> Option<String> {
    let spec = std::fs::read_to_string(dir.join(format!("{}.nuspec", id.to_lowercase()))).ok()?;
    match tag(&spec, "license") {
        Some(_) if spec.contains("type=\"file\"") => license_file(dir),
        Some(expr) => normalize(expr),
        // Older packages only link to the license.
        None => tag(&spec, "licenseUrl").and_then(normalize),
    }
}

/// The text of the first `<name>` element.
pub(super) fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    // `<license` must not match `<licenseUrl`.
    let at = xml.match_indices(&open).map(|(i, _)| i).find(|&i| xml[i + open.len()..].starts_with(['>', ' ']))?;
    let rest = &xml[at..];
    let start = rest.find('>')? + 1;
    let end = rest.find(&format!("</{name}>"))?;
    Some(rest.get(start..end)?.trim())
}

/// PackageReference lines: line number, package id, version.
fn references(project: &str) -> impl Iterator<Item = (usize, &str, &str)> {
    project.lines().enumerate().filter_map(|(line, raw)| {
        // Central package management puts versions in Directory.Packages.props as PackageVersion.
        let t = raw.trim_start();
        let rest = t.strip_prefix("<PackageReference").or_else(|| t.strip_prefix("<PackageVersion"))?;
        let attr = |name: &str| rest.split(&format!("{name}=")).nth(1).and_then(|v| quoted(v).first().copied());
        Some((line, attr("Include")?, attr("Version").unwrap_or_default()))
    })
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let lines: Vec<&str> = text.lines().collect();
    let deps = references(text).filter_map(|(line, id, _)| Mention::find(lines[line], line, id)).collect();
    let own = lines
        .iter()
        .enumerate()
        .find_map(|(line, raw)| raw.contains("<PackageLicenseExpression>").then(|| Mention::find(raw, line, "PackageLicenseExpression"))?);
    (deps, own)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start().strip_prefix("using ")?;
    let t = t.strip_prefix("static ").unwrap_or(t);
    let t = t.split_once('=').map_or(t, |(_, target)| target).trim_start();
    let ns = t.strip_suffix(';').or_else(|| t.split(';').next())?.trim();
    (ns.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '_') && !ns.is_empty()).then(|| ns.to_string())
}

#[std::prelude::rust_2024::test]
fn finds_nuget_packages() {
    let csproj = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <PackageLicenseExpression>MIT</PackageLicenseExpression>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Newtonsoft.Json\" Version=\"13.0.3\" />\n  </ItemGroup>\n</Project>\n";
    let (deps, own) = mentions(csproj);
    assert_eq!(deps.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("Newtonsoft.Json", 5)]);
    assert_eq!(own.map(|m| m.line), Some(2));
    assert_eq!(tag(csproj, "PackageLicenseExpression"), Some("MIT"));
    assert_eq!(import("using Newtonsoft.Json.Linq;").as_deref(), Some("Newtonsoft.Json.Linq"));
    assert_eq!(import("using J = Newtonsoft.Json;").as_deref(), Some("Newtonsoft.Json"));
    assert_eq!(import("using (var s = Open())"), None);
    let dir = super::scratch("nuget", &[("newtonsoft.json.nuspec", "<package><metadata><license type=\"expression\">MIT</license></metadata></package>")]);
    assert_eq!(nuspec_license(&dir, "Newtonsoft.Json").as_deref(), Some("MIT"));
    std::fs::remove_dir_all(&dir).unwrap();
}
