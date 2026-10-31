use std::path::{Path, PathBuf};

/// Moves to the Trash rather than deleting, so a slip can be undone; returns where it went.
pub fn trash(path: &Path) -> Result<PathBuf, String> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let mut moved = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut moved))
        .map_err(|e| e.localizedDescription().to_string())?;
    moved.and_then(|url| url.path()).map(|p| PathBuf::from(p.to_string())).ok_or_else(|| "Moved to the Trash".to_string())
}

/// Puts a moved entry back where it was; never over something that has taken its place.
pub fn move_back(from: &Path, to: &Path) -> Result<(), String> {
    if to.symlink_metadata().is_ok() {
        return Err(format!("“{}” already exists", to.file_name().unwrap_or_default().to_string_lossy()));
    }
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::rename(from, to).map_err(|e| e.to_string())
}

pub fn reveal(path: &Path) {
    std::process::Command::new("/usr/bin/open").arg("-R").arg(path).spawn().ok();
}

/// The path a typed name means inside `dir`. "a/b.rs" makes the folders on the way, as VS Code does.
fn target(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let name = name.trim().trim_matches('/');
    if name.is_empty() || name.split('/').any(|part| part == ".." || part == ".") {
        return Err("Enter a name".into());
    }
    let path = dir.join(name);
    if path.symlink_metadata().is_ok() {
        return Err(format!("“{name}” already exists"));
    }
    Ok(path)
}

pub fn create_file(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let path = target(dir, name)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn create_folder(dir: &Path, name: &str) -> Result<PathBuf, String> {
    let path = target(dir, name)?;
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

pub fn rename(from: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = from.parent().ok_or("Cannot rename this")?;
    if dir.join(name.trim()) == from {
        return Ok(from.to_path_buf());
    }
    // A change of case only is still a rename on a case-insensitive disk.
    let case_only = from.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name.trim()));
    let to = if case_only { dir.join(name.trim()) } else { target(dir, name)? };
    std::fs::rename(from, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

/// Moves a file or folder into `dir`, keeping its name; never over an existing entry.
pub fn move_into(from: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = from.file_name().ok_or("Cannot move this")?;
    if dir.starts_with(from) {
        return Err("A folder cannot go inside itself".into());
    }
    let to = dir.join(name);
    if to == from {
        return Ok(to);
    }
    if to.symlink_metadata().is_ok() {
        return Err(format!("“{}” already exists there", name.to_string_lossy()));
    }
    std::fs::rename(from, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

pub fn copy_into(from: &Path, dir: &Path) -> Result<PathBuf, String> {
    let name = from.file_name().ok_or("Cannot copy this")?;
    let to = dir.join(name);
    if to.symlink_metadata().is_ok() {
        return Err(format!("“{}” already exists there", name.to_string_lossy()));
    }
    copy(from, &to).map_err(|e| e.to_string())?;
    Ok(to)
}

fn copy(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

#[std::prelude::rust_2024::test]
fn moves_and_copies_without_overwriting() {
    let dir = std::env::temp_dir().join(format!("editor-move-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("a/inner")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    std::fs::write(dir.join("a/inner/x.rs"), "x").unwrap();
    std::fs::write(dir.join("y.rs"), "y").unwrap();
    assert!(move_into(&dir.join("a"), &dir.join("a/inner")).is_err(), "into itself");
    let moved = move_into(&dir.join("a"), &dir.join("b")).unwrap();
    assert_eq!(std::fs::read_to_string(moved.join("inner/x.rs")).unwrap(), "x");
    std::fs::write(dir.join("b/y.rs"), "other").unwrap();
    assert!(move_into(&dir.join("y.rs"), &dir.join("b")).is_err(), "must not overwrite");
    let copied = copy_into(&dir.join("b/a"), &dir).unwrap();
    assert_eq!(std::fs::read_to_string(copied.join("inner/x.rs")).unwrap(), "x");
    assert!(dir.join("b/a/inner/x.rs").is_file(), "the original stays");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[std::prelude::rust_2024::test]
fn creates_and_renames_without_overwriting() {
    let dir = std::env::temp_dir().join(format!("editor-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = create_file(&dir, "src/main.rs").unwrap();
    assert!(file.is_file() && dir.join("src").is_dir());
    assert_eq!(create_file(&dir, "src/main.rs"), Err("“src/main.rs” already exists".into()));
    assert!(create_file(&dir, "../escape.rs").is_err());
    assert!(create_file(&dir, "  ").is_err());
    let folder = create_folder(&dir, "docs").unwrap();
    assert!(folder.is_dir());
    std::fs::write(&file, "keep").unwrap();
    let renamed = rename(&file, "lib.rs").unwrap();
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "keep");
    assert!(rename(&renamed, "../../x.rs").is_err());
    std::fs::write(dir.join("src/other.rs"), "").unwrap();
    assert!(rename(&renamed, "other.rs").is_err(), "must not overwrite");
    assert_eq!(rename(&renamed, "Lib.rs").unwrap(), dir.join("src/Lib.rs"));
    std::fs::remove_dir_all(&dir).unwrap();
}
