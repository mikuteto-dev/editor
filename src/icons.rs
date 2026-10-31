//! File and folder icons from Material Icon Theme, matched the way VS Code matches them:
//! exact file names, then the longest extension, then the language.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use gpui_kit::{Image, ImageFormat};
use serde_json::Value;

use crate::tab::language;

struct Theme {
    dir: PathBuf,
    files: HashMap<String, String>,
    file_names: HashMap<String, String>,
    extensions: HashMap<String, String>,
    languages: HashMap<String, String>,
    folder_names: HashMap<String, String>,
    folder_names_open: HashMap<String, String>,
    file: String,
    folder: String,
    folder_open: String,
}

/// Next to the binary in Editor.app; in the source tree when run with cargo.
fn icon_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_default();
    let bundled = exe.parent().and_then(Path::parent).map(|contents| contents.join("Resources/icons"));
    let source = exe.ancestors().map(|dir| dir.join("assets/icons")).find(|dir| dir.join("material-icons.json").is_file());
    bundled.filter(|dir| dir.is_dir()).or(source).unwrap_or_default()
}

static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let dir = icon_dir();
    let manifest: Value = std::fs::read_to_string(dir.join("material-icons.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    // VS Code compares names without case.
    let map = |key: &str| -> HashMap<String, String> {
        let entries = manifest[key].as_object().into_iter().flatten();
        entries.filter_map(|(k, v)| Some((k.to_lowercase(), v.as_str()?.to_string()))).collect()
    };
    let files = manifest["iconDefinitions"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(id, def)| Some((id.clone(), def["iconPath"].as_str()?.rsplit('/').next()?.to_string())))
        .collect();
    let id = |key: &str, fallback: &str| manifest[key].as_str().unwrap_or(fallback).to_string();
    Theme {
        files,
        file_names: map("fileNames"),
        extensions: map("fileExtensions"),
        languages: map("languageIds"),
        folder_names: map("folderNames"),
        folder_names_open: map("folderNamesExpanded"),
        file: id("file", "file"),
        folder: id("folder", "folder"),
        folder_open: id("folderExpanded", "folder-open"),
        dir,
    }
});

/// Images by icon id, read from disk the first time one is shown.
static IMAGES: LazyLock<Mutex<HashMap<String, Arc<Image>>>> = LazyLock::new(Mutex::default);

fn image(id: &str) -> Arc<Image> {
    let theme = &*THEME;
    let mut images = IMAGES.lock().unwrap();
    if let Some(image) = images.get(id) {
        return image.clone();
    }
    let file = theme.files.get(id).cloned().unwrap_or_else(|| format!("{id}.svg"));
    let bytes = std::fs::read(theme.dir.join(file)).unwrap_or_default();
    let image = Arc::new(Image::from_bytes(ImageFormat::Svg, bytes));
    images.insert(id.to_string(), image.clone());
    image
}

pub fn folder(name: &str, open: bool) -> Arc<Image> {
    image(&folder_icon(name, open))
}

pub fn file(name: &str) -> Arc<Image> {
    image(&file_icon(name))
}

fn folder_icon(name: &str, open: bool) -> String {
    let theme = &*THEME;
    let (names, default) = if open { (&theme.folder_names_open, &theme.folder_open) } else { (&theme.folder_names, &theme.folder) };
    names.get(&name.to_lowercase()).unwrap_or(default).clone()
}

fn file_icon(name: &str) -> String {
    let theme = &*THEME;
    let lower = name.to_lowercase();
    if let Some(id) = theme.file_names.get(&lower) {
        return id.clone();
    }
    // The longest extension wins: "test.ts" before "ts", "d.ts" before "ts".
    let dots = lower.char_indices().filter(|(_, c)| *c == '.').map(|(i, _)| i);
    if let Some(id) = dots.filter_map(|i| theme.extensions.get(&lower[i + 1..])).next() {
        return id.clone();
    }
    let language = match language(Path::new(name)) {
        "bash" => "shellscript",
        "tsx" => "typescriptreact",
        "make" => "makefile",
        other => other,
    };
    theme.languages.get(language).unwrap_or(&theme.file).clone()
}

#[std::prelude::rust_2024::test]
fn matches_like_vs_code() {
    let cases = [
        ("main.rs", "rust"),
        ("README.md", "readme"),
        ("Cargo.toml", "toml"),
        ("package.json", "nodejs"),
        ("App.tsx", "react_ts"),
        ("app.test.ts", "test-ts"),
        ("types.d.ts", "typescript-def"),
        ("Dockerfile", "docker"),
        ("AGENTS.md", "agent"),
        ("history.rs", "rust"),
        ("report.pdf", "pdf"),
        ("data.csv", "table"),
        ("App.vue", "vue"),
        ("unknown.zzz", "file"),
    ];
    for (name, id) in cases {
        assert_eq!(file_icon(name), id, "{name}");
    }
    assert_eq!(folder_icon("src", false), "folder-src");
    assert_eq!(folder_icon("src", true), "folder-src-open");
    assert_eq!(folder_icon("whatever", false), "folder");
    // Every icon the theme names has its SVG.
    let theme = &*THEME;
    for file in theme.files.values().filter(|f| !f.contains("_light") && !f.contains("highContrast")) {
        assert!(theme.dir.join(file).is_file(), "{file}");
    }
}
