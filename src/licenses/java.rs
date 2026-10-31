use std::path::{Path, PathBuf};

use super::{Dep, Mention, Report, dotnet::tag, license_file, normalize, quoted};

struct Coordinate {
    group: String,
    artifact: String,
    version: Option<String>,
    shipped: bool,
}

pub fn report(manifest: &Path) -> Option<Report> {
    let dir = manifest.parent()?;
    let text = std::fs::read_to_string(manifest).ok()?;
    let (coordinates, own) = if manifest.extension().is_some_and(|e| e == "xml") {
        (pom_dependencies(&text), pom_license(&text))
    } else {
        (gradle_dependencies(&text), None)
    };
    let deps = coordinates
        .into_iter()
        .map(|c| {
            let pom = find_pom(&c.group, &c.artifact, c.version.as_deref());
            let version = c.version.clone().unwrap_or_default();
            // Imports name packages, which often differ from the group id (com.google.gson vs com.google.code.gson).
            let mut aliases = pom.as_deref().map(jar_packages).unwrap_or_default();
            aliases.push(c.group.clone());
            Dep { shipped: c.shipped, aliases, ..Dep::new(c.artifact, version, pom.and_then(|p| license_with_parents(&p, 3))) }
        })
        .collect();
    Some(Report::flat(own.or_else(|| license_file(dir)), deps))
}

/// The packages in the jar cached next to a POM, three levels deep ("com.google.gson").
fn jar_packages(pom: &Path) -> Vec<String> {
    let Some(dir) = pom.parent() else { return Vec::new() };
    // Gradle keeps the jar in a sibling directory named by its hash.
    let near = std::iter::once(dir.to_path_buf()).chain(dir.parent().into_iter().flat_map(|p| {
        std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path())
    }));
    let jar = near
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
        .find(|p| p.extension().is_some_and(|e| e == "jar") && !p.to_string_lossy().ends_with("-sources.jar"));
    let Some(jar) = jar else { return Vec::new() };
    let Ok(out) = std::process::Command::new("unzip").arg("-Z1").arg(&jar).output() else { return Vec::new() };
    let mut packages: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.ends_with(".class") && !l.starts_with("META-INF"))
        .filter_map(|l| {
            let parts: Vec<&str> = l.rsplit_once('/')?.0.split('/').take(3).collect();
            Some(parts.join("."))
        })
        .collect();
    packages.sort();
    packages.dedup();
    packages
}

fn pom_dependencies(pom: &str) -> Vec<Coordinate> {
    let body = pom.split("<dependencies>").nth(1).unwrap_or_default();
    body.split("<dependency>")
        .skip(1)
        .filter_map(|block| {
            let block = block.split("</dependency>").next()?;
            let scope = tag(block, "scope").unwrap_or("compile");
            Some(Coordinate {
                group: tag(block, "groupId")?.to_string(),
                artifact: tag(block, "artifactId")?.to_string(),
                // A ${property} version needs the full Maven model; any cached version stands in.
                version: tag(block, "version").filter(|v| !v.contains('$')).map(str::to_string),
                shipped: !matches!(scope, "test" | "provided"),
            })
        })
        .collect()
}

/// `group:artifact:version` strings and the configuration they are declared under.
fn gradle_lines(text: &str) -> impl Iterator<Item = (usize, &str, &str)> {
    text.lines().enumerate().filter_map(|(line, raw)| {
        let t = raw.trim_start();
        let config: &str = t.split(['(', ' ']).next()?;
        let spec = quoted(t).into_iter().find(|q| q.matches(':').count() >= 1 && !q.contains("//"))?;
        Some((line, config, spec))
    })
}

fn gradle_dependencies(text: &str) -> Vec<Coordinate> {
    gradle_lines(text)
        .filter_map(|(_, config, spec)| {
            let mut parts = spec.split(':');
            Some(Coordinate {
                group: parts.next()?.to_string(),
                artifact: parts.next()?.to_string(),
                version: parts.next().map(str::to_string),
                shipped: !(config.starts_with("test") || config == "compileOnly" || config.contains("annotationProcessor")),
            })
        })
        .collect()
}

fn find_pom(group: &str, artifact: &str, version: Option<&str>) -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    let maven = home.join(".m2/repository").join(group.replace('.', "/")).join(artifact);
    let gradle = home.join(".gradle/caches/modules-2/files-2.1").join(group).join(artifact);
    for base in [maven, gradle] {
        let versions: Vec<PathBuf> = match version {
            Some(v) => vec![base.join(v)],
            None => std::fs::read_dir(&base).into_iter().flatten().flatten().map(|e| e.path()).collect(),
        };
        for dir in versions {
            // Gradle nests each file under a directory named by its hash.
            let candidates = std::iter::once(dir.clone()).chain(std::fs::read_dir(&dir).into_iter().flatten().flatten().map(|e| e.path()));
            for candidate in candidates {
                let found = std::fs::read_dir(&candidate)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .find(|p| p.extension().is_some_and(|e| e == "pom"));
                if found.is_some() {
                    return found;
                }
            }
        }
    }
    None
}

fn pom_license(pom: &str) -> Option<String> {
    let licenses = pom.split("<licenses>").nth(1)?.split("</licenses>").next()?;
    let names: Vec<String> = licenses
        .split("<license>")
        .skip(1)
        .filter_map(|l| normalize(tag(l, "name").or_else(|| tag(l, "url"))?))
        .collect();
    (!names.is_empty()).then(|| names.join(" OR "))
}

/// Many POMs inherit their license from a parent POM.
fn license_with_parents(pom: &Path, depth: u32) -> Option<String> {
    let text = std::fs::read_to_string(pom).ok()?;
    pom_license(&text).or_else(|| {
        let parent = text.split("<parent>").nth(1)?.split("</parent>").next()?;
        let pom = find_pom(tag(parent, "groupId")?, tag(parent, "artifactId")?, tag(parent, "version"))?;
        (depth > 0).then(|| license_with_parents(&pom, depth - 1))?
    })
}

pub fn mentions(text: &str) -> (Vec<Mention>, Option<Mention>) {
    let lines: Vec<&str> = text.lines().collect();
    if text.contains("<project") {
        let start = lines.iter().position(|l| l.contains("<dependencies>")).unwrap_or(lines.len());
        let deps = lines
            .iter()
            .enumerate()
            .skip(start)
            .filter_map(|(line, raw)| Mention::find(raw, line, tag(raw, "artifactId")?))
            .collect();
        let own = lines.iter().enumerate().find_map(|(line, raw)| raw.contains("<licenses>").then(|| Mention::find(raw, line, "licenses"))?);
        return (deps, own);
    }
    let deps = gradle_lines(text)
        .filter_map(|(line, _, spec)| Mention::find(lines[line], line, spec.split(':').nth(1)?))
        .collect();
    (deps, None)
}

pub fn import(line: &str) -> Option<String> {
    let t = line.trim_start().strip_prefix("import ")?;
    let t = t.strip_prefix("static ").unwrap_or(t);
    let path = t.split([';', ' ']).next()?.trim_end_matches(".*");
    (path.contains('.') && !path.starts_with("java.") && !path.starts_with("javax.") && !path.starts_with("kotlin.")).then(|| path.to_string())
}

#[std::prelude::rust_2024::test]
fn finds_maven_and_gradle_dependencies() {
    let pom = "<project>\n  <licenses>\n    <license><name>The Apache Software License, Version 2.0</name></license>\n  </licenses>\n  <dependencies>\n    <dependency>\n      <groupId>com.google.code.gson</groupId>\n      <artifactId>gson</artifactId>\n      <version>2.10</version>\n    </dependency>\n    <dependency>\n      <groupId>junit</groupId>\n      <artifactId>junit</artifactId>\n      <scope>test</scope>\n    </dependency>\n  </dependencies>\n</project>\n";
    let deps = pom_dependencies(pom);
    let got: Vec<_> = deps.iter().map(|c| (c.artifact.as_str(), c.version.as_deref(), c.shipped)).collect();
    assert_eq!(got, [("gson", Some("2.10"), true), ("junit", None, false)]);
    assert_eq!(pom_license(pom).as_deref(), Some("Apache-2.0"));
    let (mentioned, own) = mentions(pom);
    assert_eq!(mentioned.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("gson", 7), ("junit", 12)]);
    assert_eq!(own.map(|m| m.line), Some(1));

    let gradle = "dependencies {\n    implementation(\"com.squareup.okhttp3:okhttp:4.12.0\")\n    testImplementation 'junit:junit:4.13'\n}\n";
    let got: Vec<_> = gradle_dependencies(gradle).iter().map(|c| (c.artifact.clone(), c.shipped)).collect();
    assert_eq!(got, [("okhttp".to_string(), true), ("junit".to_string(), false)]);
    assert_eq!(mentions(gradle).0.iter().map(|m| (m.name.as_str(), m.line)).collect::<Vec<_>>(), [("okhttp", 1), ("junit", 2)]);
    assert_eq!(import("import okhttp3.OkHttpClient;").as_deref(), Some("okhttp3.OkHttpClient"));
    assert_eq!(import("import java.util.List;"), None);
}
