use std::{collections::HashMap, sync::Arc, sync::LazyLock};

use gpui_kit::{Image, ImageFormat};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../assets/icons/", $name, ".svg")))),*]
    };
}

const SOURCES: &[(&str, &[u8])] = icons!(
    "authors", "bun", "c", "changelog", "citation", "claude", "conduct", "console",
    "contributing", "cpp", "credits", "csharp", "css", "database", "docker", "document",
    "editorconfig", "eslint", "folder", "gemfile", "git", "go", "go-mod", "gradle", "h", "hpp",
    "html", "image", "java", "javascript", "json", "kotlin", "license", "lock", "lua",
    "makefile", "markdown", "maven", "npm", "php", "pnpm", "prettier", "python", "react",
    "react_ts", "readme", "roadmap", "ruby", "rust", "sass", "settings", "svg", "swift", "todo",
    "toml", "tsconfig", "typescript", "video", "vite", "xml", "yaml", "yarn",
);

static IMAGES: LazyLock<HashMap<&str, Arc<Image>>> = LazyLock::new(|| {
    SOURCES
        .iter()
        .map(|(name, svg)| (*name, Arc::new(Image::from_bytes(ImageFormat::Svg, svg.to_vec()))))
        .collect()
});

pub fn folder() -> Arc<Image> {
    IMAGES["folder"].clone()
}

pub fn file(name: &str) -> Arc<Image> {
    IMAGES[file_icon(name)].clone()
}

fn file_icon(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    // Well-known docs keep their meaning whatever the extension (README, README.md, readme.txt).
    let stem = lower.split('.').next().unwrap_or_default();
    let by_stem = match stem {
        "readme" => Some("readme"),
        "changelog" | "changes" | "history" => Some("changelog"),
        "contributing" => Some("contributing"),
        s if s.starts_with("license") || s.starts_with("licence") || s == "copying" => Some("license"),
        "code_of_conduct" => Some("conduct"),
        "todo" => Some("todo"),
        "authors" | "contributors" => Some("authors"),
        "claude" => Some("claude"),
        "roadmap" => Some("roadmap"),
        "credits" => Some("credits"),
        "citation" => Some("citation"),
        "vite" if lower.starts_with("vite.config.") => Some("vite"),
        "eslint" if lower.starts_with("eslint.config.") => Some("eslint"),
        _ => None,
    };
    if let Some(icon) = by_stem {
        return icon;
    }
    match lower.as_str() {
        "dockerfile" | "compose.yml" | "compose.yaml" | "docker-compose.yml" => return "docker",
        ".gitignore" | ".gitattributes" | ".gitmodules" => return "git",
        "package.json" | "package-lock.json" | ".npmrc" => return "npm",
        "pnpm-lock.yaml" | "pnpm-workspace.yaml" => return "pnpm",
        "bun.lock" | "bun.lockb" | "bunfig.toml" => return "bun",
        "yarn.lock" | ".yarnrc" | ".yarnrc.yml" => return "yarn",
        "go.mod" | "go.sum" => return "go-mod",
        "tsconfig.json" => return "tsconfig",
        ".prettierrc" | ".prettierrc.json" | "prettier.config.js" => return "prettier",
        ".eslintrc" | ".eslintrc.json" | ".eslintrc.js" => return "eslint",
        ".editorconfig" => return "editorconfig",
        "makefile" | "gnumakefile" => return "makefile",
        "gemfile" | "gemfile.lock" => return "gemfile",
        "build.gradle" | "build.gradle.kts" | "settings.gradle" => return "gradle",
        "pom.xml" => return "maven",
        _ => {}
    }
    let ext = lower.rsplit_once('.').map_or("", |(_, ext)| ext);
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "react",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "react_ts",
        "go" => "go",
        "java" => "java",
        "c" => "c",
        "cpp" | "cc" | "cxx" => "cpp",
        "h" => "h",
        "hpp" | "hh" => "hpp",
        "cs" => "csharp",
        "css" => "css",
        "scss" | "sass" => "sass",
        "html" | "htm" => "html",
        "json" | "jsonc" => "json",
        "md" | "mdx" => "markdown",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "php" => "php",
        "rb" => "ruby",
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "lua" => "lua",
        "sh" | "bash" | "zsh" | "fish" => "console",
        "sql" => "database",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" | "tif" | "tiff" => "image",
        "mp4" | "mov" | "m4v" | "webm" | "mkv" => "video",
        "svg" => "svg",
        "xml" => "xml",
        "lock" => "lock",
        "ini" | "conf" | "cfg" | "env" => "settings",
        _ => "document",
    }
}

#[test]
fn file_icons() {
    assert_eq!(file_icon("main.rs"), "rust");
    assert_eq!(file_icon("README.md"), "readme");
    assert_eq!(file_icon("App.TSX"), "react_ts");
    assert_eq!(file_icon("Dockerfile"), "docker");
    assert_eq!(file_icon("Makefile"), "makefile");
    assert_eq!(file_icon(".env"), "settings");
    assert_eq!(file_icon("CHANGELOG.md"), "changelog");
    assert_eq!(file_icon("LICENSE-APACHE"), "license");
    assert_eq!(file_icon("CODE_OF_CONDUCT.md"), "conduct");
    assert_eq!(file_icon("CLAUDE.md"), "claude");
    assert_eq!(file_icon("AGENTS.md"), "markdown");
    assert_eq!(file_icon("vite.config.ts"), "vite");
    assert_eq!(file_icon("package.json"), "npm");
    // Every mapped name must exist, or `IMAGES[..]` panics at runtime.
    let names = "a.rs a.py a.js a.jsx a.ts a.tsx a.go a.java a.c a.cpp a.h a.hpp a.cs a.css a.scss \
                 a.html a.json a.md a.yml a.toml a.php a.rb a.swift a.kt a.lua a.sh a.sql a.png \
                 a.svg a.xml a.lock a.ini a.mp4 README CHANGES LICENSE CODE_OF_CONDUCT.md TODO \
                 AUTHORS CLAUDE.md ROADMAP.md CREDITS CITATION.cff vite.config.js eslint.config.js \
                 package.json pnpm-lock.yaml bun.lock yarn.lock go.mod tsconfig.json .prettierrc \
                 .eslintrc .editorconfig Makefile Gemfile build.gradle pom.xml Dockerfile .gitignore";
    for name in names.split_whitespace() {
        assert!(IMAGES.contains_key(file_icon(name)), "{name}");
    }
}
