mod commands;
mod dep_graph;
mod diff_view;
mod git;
mod git_panel;
mod icons;
mod licenses;
mod palette;
mod recent;
mod search;
mod session;
mod shell;
mod tab;
mod theme;
mod tree;
mod video;
mod watch;
mod workspace;

use std::path::PathBuf;

use gpui_kit::{
    component::TitleBar,
    *,
};

fn config_dir() -> PathBuf {
    std::env::home_dir()
        .unwrap_or_default()
        .join("Library/Application Support/editor")
}

fn main() {
    shell::warm();
    let (opened, open_requests) = smol::channel::unbounded::<PathBuf>();
    let arg = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .map(|path| path.canonicalize().unwrap_or(path));
    // Like other editors: no path resumes the last folder, and with no history nothing opens.
    let root = match arg {
        Some(dir) if dir.is_dir() => Some(dir),
        Some(file) => {
            opened.try_send(file).ok();
            None
        }
        None => recent::load().into_iter().find(|dir| dir.is_dir()),
    };

    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);
    // Finder double-clicks and drops on the Dock icon arrive as file URLs.
    app.on_open_urls(move |urls| {
        for path in urls.iter().filter_map(|url| file_path(url)) {
            opened.try_send(path).ok();
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        theme::init(cx);
        tree::init(cx);
        git_panel::init(cx);
        commands::init(cx);
        cx.activate(true);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1200.), px(800.)), cx)),
            ..TitleBar::window_options()
        };
        let (window, workspace) = gpui_kit::open_window(options, cx, |window, cx| {
            let workspace = cx.new(|cx| workspace::Workspace::new(root, window, cx));
            // Open what is already queued before the first frame, so it never flashes empty.
            while let Ok(path) = open_requests.try_recv() {
                workspace.update(cx, |w, cx| w.open_path(path, window, cx));
            }
            workspace
        })
        .expect("open window");
        cx.spawn(async move |cx| {
            while let Ok(path) = open_requests.recv().await {
                window
                    .update(cx, |_, window, cx| {
                        cx.activate(true);
                        workspace.update(cx, |w, cx| w.open_path(path, window, cx));
                    })
                    .ok();
            }
        })
        .detach();
    });
}

fn file_path(url: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut bytes = url.strip_prefix("file://")?.bytes();
    let mut path = Vec::new();
    while let Some(b) = bytes.next() {
        path.push(match b {
            b'%' => {
                let hex = [bytes.next()?, bytes.next()?];
                u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?
            }
            b => b,
        });
    }
    Some(std::ffi::OsString::from_vec(path).into())
}

#[std::prelude::rust_2024::test]
fn file_urls() {
    assert_eq!(file_path("file:///a/b%20c.rs"), Some(PathBuf::from("/a/b c.rs")));
    assert_eq!(file_path("file:///%E3%81%82.md"), Some(PathBuf::from("/あ.md")));
    assert_eq!(file_path("https://x"), None);
    assert_eq!(file_path("file:///bad%2"), None);
}
