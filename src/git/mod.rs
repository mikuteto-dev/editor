//! Git through the `git` CLI: one process per command, nothing resident.

mod blame;
mod read;
mod write;

use std::path::Path;

pub use blame::blame;
pub use read::*;
pub use write::*;

use crate::shell;

fn git(dir: &Path, args: &[&str], ok: &[i32]) -> Option<String> {
    let out = shell::command("git").arg("-C").arg(dir).args(args).output().ok()?;
    ok.contains(&out.status.code()?)
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}
