use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use super::Engine;

struct Tools {
    latexmk: PathBuf,
    bins: Vec<PathBuf>,
    trees: Vec<PathBuf>,
}

fn find(name: &str) -> Result<PathBuf, String> {
    crate::shell::executable(name)
        .or_else(|| {
            let path = Path::new("/Library/TeX/texbin").join(name);
            path.is_file().then_some(path)
        })
        .ok_or_else(|| format!("{name} not found. Install MacTeX/TeX Live with latexmk and the selected engine, then restart Editor."))
}

impl Tools {
    fn find(engine: Engine) -> Result<Self, String> {
        let mut programs = vec![find("latexmk")?, find(engine.program())?];
        if engine == Engine::Platex {
            programs.push(find("dvipdfmx")?);
        }
        if engine == Engine::Xe {
            programs.push(find("xdvipdfmx")?);
        }
        let mut bins = Vec::new();
        let mut trees = Vec::new();
        for program in &programs {
            if let Some(bin) = program.parent() {
                bins.push(bin.to_path_buf());
            }
            let canonical = program.canonicalize().map_err(|e| e.to_string())?;
            if let Some(tree) = canonical
                .ancestors()
                .find(|path| path.join("tlpkg/texlive.tlpdb").is_file())
            {
                trees.push(tree.to_path_buf());
            }
        }
        bins.dedup();
        trees.sort();
        trees.dedup();
        Ok(Self {
            latexmk: programs.remove(0),
            bins,
            trees,
        })
    }
}

fn arguments(engine: Engine, output: &Path, input: &Path) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = [
        "-norc",
        engine.mode(),
        "-interaction=nonstopmode",
        "-halt-on-error",
        "-file-line-error",
        "-latexoption=-no-shell-escape",
        "-e",
        "$max_repeat=5;",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if engine == Engine::Platex {
        args.extend([
            "-e".into(),
            "$latex='platex -no-shell-escape %O %S'; $dvipdf='dvipdfmx %O -o %D %S';".into(),
        ]);
    }
    let mut dir = std::ffi::OsString::from("-outdir=");
    dir.push(output);
    args.push(dir);
    args.push(input.as_os_str().to_owned());
    args
}

pub fn compile(
    path: &Path,
    root: &Path,
    source: &str,
    engine: Engine,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    if source.len() > 4 * 1024 * 1024 {
        return Err("TeX source exceeds 4 MiB".into());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Build cancelled".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let dir = path
        .parent()
        .ok_or("TeX file has no directory")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if root.parent().is_none() || !dir.starts_with(&root) {
        return Err("TeX file must be inside the open project folder".into());
    }
    let tools = Tools::find(engine)?;
    let work = tempfile::Builder::new()
        .prefix("editor-tex-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let work_path = work.path().canonicalize().map_err(|e| e.to_string())?;
    for name in ["input", "output", "home", "cache", "tmp"] {
        fs::create_dir(work_path.join(name)).map_err(|e| e.to_string())?;
    }
    let stem = path.file_stem().ok_or("TeX file has no name")?;
    let mut input_name = stem.to_os_string();
    input_name.push(".tex");
    let input = work_path.join("input").join(input_name);
    let output = work_path.join("output");
    fs::write(&input, source).map_err(|e| e.to_string())?;
    let profile = profile(
        &root,
        &work_path,
        &tools.trees,
        std::slice::from_ref(&tools.latexmk),
    )?;
    let mut command = Command::new("/usr/bin/sandbox-exec");
    command
        .args(["-p", &profile])
        .arg(&tools.latexmk)
        .args(arguments(engine, &output, &input))
        .current_dir(&dir);
    // TeX is executable input. It must not inherit API keys, shell startup files or user rc files.
    let mut bins = tools.bins;
    bins.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/sbin"),
    ]);
    command
        .env_clear()
        .env(
            "PATH",
            std::env::join_paths(bins).map_err(|e| e.to_string())?,
        )
        .env("HOME", work_path.join("home"))
        .env("USER", "tex")
        .env("TMPDIR", work_path.join("tmp"))
        .env("XDG_CACHE_HOME", work_path.join("cache"))
        .env("TEXMFVAR", work_path.join("cache/texmf-var"))
        .env("TEXMFCONFIG", work_path.join("cache/texmf-config"))
        .env("TEXMFCACHE", work_path.join("cache"))
        .env("BIBER_CACHE", work_path.join("cache"))
        .env("PAR_GLOBAL_TMPDIR", work_path.join("cache"))
        .env("TEXMFOUTPUT", &output)
        // Absolute cache writes are confined by the OS sandbox, not kpathsea.
        .env("openout_any", "a")
        .env("LC_CTYPE", "UTF-8");
    let log = work_path.join("build.log");
    run(&mut command, &log, cancel, Duration::from_secs(120))?;
    let mut pdf_name = stem.to_os_string();
    pdf_name.push(".pdf");
    let pdf = output.join(pdf_name);
    let size = fs::metadata(&pdf)
        .map_err(|_| "TeX did not produce a PDF".to_string())?
        .len();
    if size > 64 * 1024 * 1024 {
        return Err("Generated PDF exceeds 64 MiB".into());
    }
    fs::read(pdf).map_err(|e| e.to_string())
}

fn profile(
    root: &Path,
    work: &Path,
    trees: &[PathBuf],
    executables: &[PathBuf],
) -> Result<String, String> {
    let quoted = |path: &Path| {
        serde_json::to_string(path.to_str().ok_or("TeX paths must be valid Unicode")?)
            .map_err(|e| e.to_string())
    };
    let mut reads: Vec<PathBuf> = [
        "/System",
        "/usr",
        "/bin",
        "/sbin",
        "/Library/TeX",
        "/Library/Fonts",
        "/opt/homebrew",
        "/dev",
        "/private/var/db/dyld",
        "/private/var/db/fonts",
        "/private/var/db/timezone",
        "/private/etc",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    reads.extend([root.to_path_buf(), work.to_path_buf()]);
    reads.extend_from_slice(trees);
    // dyld opens the root directory; a literal rule does not grant reads of its descendants.
    let mut profile = String::from(
        "(version 1)\n(allow default)\n(deny network*)\n(deny appleevent-send)\n(deny file-read-data)\n(deny file-write*)\n(deny process-exec)\n(allow file-read-data (literal \"/\"))\n",
    );
    for path in reads {
        profile.push_str(&format!(
            "(allow file-read-data (subpath {}))\n",
            quoted(&path)?
        ));
    }
    profile.push_str(&format!(
        "(allow file-write* (subpath {})(literal \"/dev/null\"))\n(deny file-link)\n",
        quoted(work)?
    ));
    for path in [
        "/usr/bin/perl",
        "/usr/bin/env",
        "/bin/sh",
        "/bin/bash",
        "/usr/bin/dirname",
        "/usr/bin/basename",
        "/bin/echo",
        "/bin/mkdir",
        "/bin/rm",
        "/bin/cat",
        "/usr/bin/touch",
        "/usr/bin/sed",
        "/usr/bin/grep",
        "/usr/bin/tr",
        "/usr/bin/awk",
        "/usr/bin/sort",
        "/usr/bin/uname",
        "/bin/date",
        "/usr/bin/cmp",
        "/usr/bin/head",
        "/usr/bin/tail",
        "/usr/bin/cut",
        "/usr/bin/which",
        "/usr/bin/printf",
        "/usr/bin/wc",
        "/bin/ls",
    ] {
        profile.push_str(&format!("(allow process-exec (literal {path:?}))\n"));
    }
    for path in [
        Path::new("/Library/TeX"),
        Path::new("/usr/local/texlive"),
        Path::new("/opt/homebrew"),
    ]
    .into_iter()
    .chain(trees.iter().map(PathBuf::as_path))
    {
        profile.push_str(&format!(
            "(allow process-exec (subpath {}))\n",
            quoted(path)?
        ));
    }
    for path in executables {
        profile.push_str(&format!(
            "(allow process-exec (literal {}))\n",
            quoted(&path.canonicalize().map_err(|e| e.to_string())?)?
        ));
    }
    Ok(profile)
}

fn run(
    command: &mut Command,
    log: &Path,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<(), String> {
    let file = File::create(log).map_err(|e| e.to_string())?;
    command
        .stdin(Stdio::null())
        .stdout(file.try_clone().map_err(|e| e.to_string())?)
        .stderr(file)
        .process_group(0);
    // Only async-signal-safe syscalls are used between fork and exec.
    unsafe {
        command.pre_exec(|| {
            for (resource, limit) in [
                (libc::RLIMIT_CPU, 90),
                (libc::RLIMIT_FSIZE, 64 * 1024 * 1024),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let result = loop {
        if cancel.load(Ordering::Acquire) {
            break Err("Build cancelled".into());
        }
        if start.elapsed() >= timeout {
            break Err("TeX build exceeded 120 seconds".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                break if status.success() {
                    Ok(())
                } else {
                    Err(log_tail(log))
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(error) => break Err(error.to_string()),
        }
    };
    // Kill the whole job, including helpers or descendants left behind after latexmk exits.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    _ = child.wait();
    result
}

fn log_tail(path: &Path) -> String {
    let text = (|| -> std::io::Result<String> {
        let mut file = File::open(path)?;
        let size = file.metadata()?.len();
        file.seek(SeekFrom::Start(size.saturating_sub(16_384)))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    })()
    .unwrap_or_default();
    if text.trim().is_empty() {
        "TeX build failed".into()
    } else {
        text
    }
}

#[cfg(test)]
mod tests;
