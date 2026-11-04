use super::{arguments, profile, run};
use crate::tex::Engine;
use std::{
    fs,
    os::unix::fs::symlink,
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
fn selects_each_engine_without_user_rc_or_shell_escape() {
    for engine in Engine::ALL {
        let args = arguments(
            engine,
            std::path::Path::new("/build with spaces"),
            std::path::Path::new("/input with spaces/document.tex"),
        );
        assert!(args.iter().any(|arg| arg == "-norc"));
        assert!(
            args.iter()
                .any(|arg| arg == "-latexoption=-no-shell-escape")
        );
        assert!(args.iter().any(|arg| arg == engine.mode()));
        assert_eq!(args.last().unwrap(), "/input with spaces/document.tex");
        if engine == Engine::Platex {
            assert!(
                args.iter()
                    .any(|arg| arg.to_string_lossy().contains("dvipdfmx"))
            );
        }
    }
}

#[test]
fn sandbox_confines_reads_writes_links_and_network() {
    let root = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let inside = root.path().join("inside.txt");
    let secret = outside.path().join("outside.txt");
    fs::write(&inside, "project").unwrap();
    fs::write(&secret, "unchanged").unwrap();
    let rules = profile(
        &root.path().canonicalize().unwrap(),
        &work.path().canonicalize().unwrap(),
        &[],
        &[],
    )
    .unwrap();
    let sandbox = |program: &str, args: &[&std::ffi::OsStr]| {
        Command::new("/usr/bin/sandbox-exec")
            .args(["-p", &rules, program])
            .args(args)
            .output()
            .unwrap()
    };
    let read = sandbox("/bin/cat", &[inside.as_os_str()]);
    assert!(
        read.status.success(),
        "{}",
        String::from_utf8_lossy(&read.stderr)
    );
    assert_eq!(read.stdout, b"project");
    assert!(!sandbox("/bin/cat", &[secret.as_os_str()]).status.success());
    let link = root.path().join("link");
    symlink(&secret, &link).unwrap();
    assert!(!sandbox("/bin/cat", &[link.as_os_str()]).status.success());
    assert!(
        !sandbox(
            "/bin/sh",
            &[
                "-c".as_ref(),
                "printf lost > \"$1\"".as_ref(),
                "sh".as_ref(),
                secret.as_os_str()
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read_to_string(&secret).unwrap(), "unchanged");
    let target = work.path().join("output.txt");
    assert!(
        sandbox(
            "/bin/sh",
            &[
                "-c".as_ref(),
                "printf output > \"$1\"".as_ref(),
                "sh".as_ref(),
                target.as_os_str()
            ]
        )
        .status
        .success()
    );
    assert_eq!(fs::read_to_string(target).unwrap(), "output");
    let hardlink = work.path().join("hardlink");
    assert!(
        !sandbox(
            "/usr/bin/perl",
            &[
                "-e".as_ref(),
                "link($ARGV[0],$ARGV[1]) or exit 1".as_ref(),
                inside.as_os_str(),
                hardlink.as_os_str()
            ]
        )
        .status
        .success()
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    assert!(!sandbox("/usr/bin/perl", &["-MSocket".as_ref(), "-e".as_ref(), "socket(S,PF_INET,SOCK_STREAM,0) && connect(S,sockaddr_in($ARGV[0],inet_aton('127.0.0.1'))) or exit 1".as_ref(), port.as_ref()]).status.success());
}

#[test]
fn times_out_and_cancels_the_process_group() {
    let work = tempfile::tempdir().unwrap();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "sleep 30 & wait"]);
    let start = Instant::now();
    assert!(
        run(
            &mut command,
            &work.path().join("timeout.log"),
            &AtomicBool::new(false),
            Duration::from_millis(40)
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(3));
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "sleep 30 & wait"]);
    assert!(
        run(
            &mut command,
            &work.path().join("cancel.log"),
            &AtomicBool::new(true),
            Duration::from_secs(30)
        )
        .unwrap_err()
        .contains("cancelled")
    );
}

#[test]
fn failures_keep_the_log_but_bound_its_size() {
    let work = tempfile::tempdir().unwrap();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "printf 'Undefined control sequence'; exit 1"]);
    let error = run(
        &mut command,
        &work.path().join("error.log"),
        &AtomicBool::new(false),
        Duration::from_secs(2),
    )
    .unwrap_err();
    assert!(error.contains("Undefined control sequence"));
    let path = work.path().join("long.log");
    fs::write(&path, "x".repeat(100_000)).unwrap();
    assert_eq!(super::log_tail(&path).len(), 16_384);
}
