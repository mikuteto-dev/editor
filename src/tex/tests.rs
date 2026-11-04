use super::{Engine, Preview};
use gpui::AppContext;
use gpui_kit::component::input::EditorState;
use std::sync::Arc;

#[test]
fn engine_directives_are_explicit_and_never_commands() {
    assert_eq!(Engine::from_source("no directive").unwrap(), Engine::Lua);
    for (source, expected) in [
        ("% !TeX program = lualatex", Engine::Lua),
        ("  % !TEX program=XeLaTeX\n", Engine::Xe),
        ("% !TEX TS-program = platex", Engine::Platex),
    ] {
        assert_eq!(Engine::from_source(source).unwrap(), expected);
    }
    for value in [
        "pdflatex",
        "sh -c anything",
        "lualatex; anything",
        "/bin/sh",
    ] {
        assert!(Engine::from_source(&format!("% !TeX program = {value}")).is_err());
    }
}

#[test]
fn tex_files_are_previewable() {
    for path in ["main.tex", "MAIN.TEX", "main.ltx"] {
        assert!(matches!(
            crate::tab::PreviewKind::of(std::path::Path::new(path)),
            Some(crate::tab::PreviewKind::Tex)
        ));
    }
}

#[gpui::test]
fn preview_changes_pdf_pages_without_recompiling(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_kit::init);
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("main.tex");
    let (view, cx) = cx.add_window_view(|window, cx| {
        let editor =
            cx.new(|cx| EditorState::new(window, cx).default_value("% !TEX program=not-supported"));
        let mut preview = Preview::new(&path, root.path(), editor, cx);
        let bytes = Arc::new(super::pdf::tests::fixture());
        preview.pages = 2;
        preview.image = Some(super::pdf::render(&bytes, 0).unwrap());
        preview.bytes = Some(bytes);
        preview
    });
    view.update(cx, |view, cx| view.show(1, cx));
    cx.run_until_parked();
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
    view.read_with(cx, |view, _| {
        assert!(!view.busy);
        assert_eq!(view.page, 1);
        assert_eq!(
            (
                view.image.as_ref().unwrap().width,
                view.image.as_ref().unwrap().height
            ),
            (200, 100)
        );
    });
    view.update(cx, |view, cx| view.show(2, cx));
    assert_eq!(view.read_with(cx, |view, _| view.page), 1);
}

fn real_engine(engine: Engine, file: &str) {
    let fixtures = std::env::current_dir().unwrap().join("tests/fixtures/tex");
    let project = tempfile::Builder::new()
        .prefix("editor tex project ")
        .tempdir()
        .unwrap();
    let root = project.path().to_path_buf();
    let path = root.join("main file.tex");
    let source = std::fs::read_to_string(fixtures.join(file)).unwrap();
    std::fs::write(&path, &source).unwrap();
    std::fs::copy(fixtures.join("part.tex"), root.join("part.tex")).unwrap();
    let bytes = super::build::compile(
        &path,
        &root,
        &source,
        engine,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(super::pdf::pages(&bytes).unwrap(), 2);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
    let page = super::pdf::render(&bytes, 0).unwrap();
    assert!(
        page.image
            .as_bytes(0)
            .unwrap()
            .chunks_exact(4)
            .filter(|pixel| pixel[0] < 220 && pixel[1] < 220 && pixel[2] < 220)
            .count()
            > 100
    );
}

#[test]
#[ignore = "requires MacTeX/TeX Live with LuaLaTeX"]
fn real_sandbox_confines_lua_and_ignores_project_rc() {
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sentinel = project.path().join("sentinel.txt");
    let secret = outside.path().join("outside.txt");
    std::fs::write(&sentinel, "unchanged").unwrap();
    std::fs::write(&secret, "outside").unwrap();
    std::fs::write(
        project.path().join(".latexmkrc"),
        "die 'project rc must not run';",
    )
    .unwrap();
    let source = format!(
        r"\documentclass{{article}}
\begin{{document}}
\directlua{{
assert(os.execute() == 0)
assert(io.open([[{}]], 'w') == nil)
assert(io.open([[{}]], 'r') == nil)
assert(not lfs.link([[{}]], os.getenv('TEXMFOUTPUT') .. '/hardlink'))
}}
Sandbox verified.
\end{{document}}",
        sentinel.display(),
        secret.display(),
        sentinel.display()
    );
    let path = project.path().join("main.tex");
    std::fs::write(&path, &source).unwrap();
    let bytes = super::build::compile(
        &path,
        project.path(),
        &source,
        Engine::Lua,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(super::pdf::pages(&bytes).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(sentinel).unwrap(), "unchanged");
}

#[test]
#[ignore = "requires MacTeX/TeX Live with LuaLaTeX and Japanese packages"]
fn real_lualatex() {
    real_engine(Engine::Lua, "lua.tex");
}

#[test]
#[ignore = "requires MacTeX/TeX Live with XeLaTeX and Japanese packages"]
fn real_xelatex() {
    real_engine(Engine::Xe, "xe.tex");
}

#[test]
#[ignore = "requires MacTeX/TeX Live with pLaTeX and dvipdfmx"]
fn real_platex() {
    real_engine(Engine::Platex, "platex.tex");
}
