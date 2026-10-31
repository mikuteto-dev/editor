use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// An app opened from Finder gets launchd's minimal PATH, without Homebrew or ~/.cargo/bin;
/// the login shell's PATH is the one the user's tools (cargo, git's credential helpers) expect.
fn path() -> &'static OsString {
    static PATH: OnceLock<OsString> = OnceLock::new();
    PATH.get_or_init(|| {
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
        let login = Command::new(shell)
            .args(["-l", "-c", "printf %s \"$PATH\""])
            .output()
            .ok()
            .filter(|o| o.status.success() && !o.stdout.is_empty())
            .map(|o| OsString::from(String::from_utf8_lossy(&o.stdout).into_owned()));
        login.unwrap_or_else(|| std::env::var_os("PATH").unwrap_or_default())
    })
}

/// Asks the login shell once, off the main thread, so the first command does not wait for it.
pub fn warm() {
    std::thread::spawn(path);
}

fn find(program: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path).map(|dir| dir.join(program)).find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

pub fn command(program: &str) -> Command {
    let path = path();
    let mut command = Command::new(find(program, path).unwrap_or_else(|| program.into()));
    command.env("PATH", path);
    command
}

#[std::prelude::rust_2024::test]
fn finds_programs_on_a_path() {
    let path = OsString::from("/nonexistent:/bin:/usr/bin");
    assert_eq!(find("sh", &path), Some(PathBuf::from("/bin/sh")));
    assert_eq!(find("no-such-program", &path), None);
}
