//! Kept apart from `use gpui_kit::*`, whose `test` would make `#[gpui::test]` expand into itself.

use std::path::PathBuf;

use gpui::{Entity, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TestAppContext, VisualTestContext, point, px};

use super::{FileTree, init};

/// A folder holding "sub/" and "a.txt", shown in a tree. Under the 28px header the 24px rows are
/// "sub" centered at y 40 and "a.txt" at y 64.
fn open<'a>(name: &str, cx: &'a mut TestAppContext) -> (PathBuf, Entity<FileTree>, &'a mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("editor-tree-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(cx);
    });
    let root = dir.clone();
    let (tree, cx) = cx.add_window_view(move |_, cx| FileTree::new(root, cx));
    draw(cx);
    (dir, tree, cx)
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        _ = window.draw(cx);
    });
}

fn press(button: MouseButton, y: f32, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent { button, position: point(px(60.), px(y)), modifiers: Default::default(), click_count: 1, first_mouse: false });
}

#[gpui::test]
fn right_click_creates_a_file(cx: &mut TestAppContext) {
    let (dir, tree, cx) = open("menu", cx);
    press(MouseButton::Right, 64., cx);
    draw(cx);
    cx.simulate_keystrokes("down enter");
    draw(cx);
    assert!(tree.read_with(cx, |tree, _| tree.editing.is_some()), "New File… did not start");
    cx.simulate_input("made.rs");
    cx.simulate_keystrokes("enter");
    draw(cx);
    let made = dir.join("made.rs").is_file();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(made, "the file was not made");
}

/// The menu must be drawn, not only focused: click its first item with the mouse. Run both bare
/// and the way the app embeds the tree, in a resizable sidebar under gpui-kit's Root.
#[gpui::test]
fn right_click_menu_is_drawn_bare(cx: &mut TestAppContext) {
    let (dir, tree, cx) = open("drawn-bare", cx);
    let made = click_new_file_with_mouse(&dir, &tree, cx, 0.);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(made, "the menu was not drawn where it can be clicked");
}

#[gpui::test]
fn right_click_menu_is_drawn_in_the_sidebar(cx: &mut TestAppContext) {
    use gpui::{AppContext as _, AnyView, Context, IntoElement, ParentElement, Render, Styled, Window, div};
    use gpui_kit::component::resizable::{h_resizable, resizable_panel};

    struct Host(AnyView);
    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            h_resizable("main-side")
                .child(resizable_panel().size(px(260.)).child(self.0.clone()))
                .child(resizable_panel().child(div().size_full()))
        }
    }

    let dir = std::env::temp_dir().join(format!("editor-tree-drawn-host-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        init(cx);
    });
    let root = dir.clone();
    let slot = std::rc::Rc::new(std::cell::RefCell::new(None));
    let built = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let tree = cx.new(|cx| FileTree::new(root, cx));
        *built.borrow_mut() = Some(tree.clone());
        let host = cx.new(|_| Host(tree.into()));
        gpui_kit::base::Root::new(host, window, cx)
    });
    draw(cx);
    let tree = slot.borrow().clone().unwrap();
    let made = click_new_file_with_mouse(&dir, &tree, cx, 0.);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(made, "inside the sidebar, the menu was not drawn where it can be clicked");
}

fn click_new_file_with_mouse(dir: &std::path::Path, tree: &Entity<FileTree>, cx: &mut VisualTestContext, top: f32) -> bool {
    let at = point(px(60.), px(top + 64.));
    cx.simulate_event(MouseDownEvent { button: MouseButton::Right, position: at, modifiers: Default::default(), click_count: 1, first_mouse: false });
    cx.simulate_event(MouseUpEvent { button: MouseButton::Right, position: at, modifiers: Default::default(), click_count: 1 });
    draw(cx);
    // The menu opens with its top-left corner at the pointer; its first item is just inside.
    let item = point(px(60. + 30.), px(top + 64. + 14.));
    cx.simulate_event(MouseMoveEvent { position: item, pressed_button: None, modifiers: Default::default() });
    draw(cx);
    cx.simulate_event(MouseDownEvent { button: MouseButton::Left, position: item, modifiers: Default::default(), click_count: 1, first_mouse: false });
    cx.simulate_event(MouseUpEvent { button: MouseButton::Left, position: item, modifiers: Default::default(), click_count: 1 });
    draw(cx);
    if !tree.read_with(cx, |tree, _| tree.editing.is_some()) {
        return false;
    }
    cx.simulate_input("made.rs");
    cx.simulate_keystrokes("enter");
    draw(cx);
    dir.join("made.rs").is_file()
}

/// Right-clicking the empty space below the rows offers the root's menu.
#[gpui::test]
fn right_click_below_the_rows_creates_in_the_root(cx: &mut TestAppContext) {
    let (dir, tree, cx) = open("blank", cx);
    // Two rows end at y 76; 200 is empty space.
    let made = click_new_file_with_mouse(&dir, &tree, cx, 200. - 64.);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(made, "no menu on the empty space");
}

/// Dragging "a.txt": passing over the closed "sub" folder leaves it shut; resting on it opens it.
#[gpui::test]
fn resting_a_drag_on_a_folder_opens_it(cx: &mut TestAppContext) {
    let (dir, tree, cx) = open("spring", cx);
    std::fs::write(dir.join("sub/inner.txt"), "").unwrap();
    let opened = |cx: &mut VisualTestContext| tree.read_with(cx, |tree, _| tree.rows.iter().any(|r| r.name == "inner.txt"));
    let move_to = |y: f32, cx: &mut VisualTestContext| {
        cx.simulate_event(MouseMoveEvent { position: point(px(60.), px(y)), pressed_button: Some(MouseButton::Left), modifiers: Default::default() });
        draw(cx);
    };
    press(MouseButton::Left, 64., cx);
    move_to(52., cx);
    move_to(40., cx);
    cx.executor().advance_clock(std::time::Duration::from_millis(300));
    move_to(64., cx);
    cx.executor().advance_clock(std::time::Duration::from_millis(1000));
    draw(cx);
    let passed_over = opened(cx);
    // A hand on a trackpad never holds quite still: keep nudging within the folder.
    let mut blinked = false;
    for y in [40., 41., 39., 40., 42., 41., 40., 39., 40.] {
        move_to(y, cx);
        cx.executor().advance_clock(std::time::Duration::from_millis(100));
        draw(cx);
        blinked |= tree.read_with(cx, |tree, _| tree.spring.blinking.is_some());
    }
    draw(cx);
    let rested = opened(cx);
    assert!(blinked, "the folder did not blink before opening");
    cx.simulate_event(MouseUpEvent { button: MouseButton::Left, position: point(px(60.), px(300.)), modifiers: Default::default(), click_count: 1 });
    draw(cx);
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(!passed_over, "a folder opened while the drag only passed over it");
    assert!(rested, "the folder did not open while the drag rested on it");
}

#[gpui::test]
fn dragging_a_file_onto_a_folder_moves_it(cx: &mut TestAppContext) {
    let (dir, _, cx) = open("drag", cx);
    press(MouseButton::Left, 64., cx);
    for y in [60., 52., 44., 40.] {
        cx.simulate_event(MouseMoveEvent { position: point(px(60.), px(y)), pressed_button: Some(MouseButton::Left), modifiers: Default::default() });
        draw(cx);
    }
    cx.simulate_event(MouseUpEvent { button: MouseButton::Left, position: point(px(60.), px(40.)), modifiers: Default::default(), click_count: 1 });
    draw(cx);
    let moved = dir.join("sub/a.txt").is_file() && !dir.join("a.txt").exists();
    cx.simulate_keystrokes("cmd-z");
    draw(cx);
    let undone = dir.join("a.txt").is_file() && !dir.join("sub/a.txt").exists();
    cx.simulate_keystrokes("cmd-shift-z");
    draw(cx);
    let redone = dir.join("sub/a.txt").is_file() && !dir.join("a.txt").exists();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(moved, "a.txt did not move into sub");
    assert!(undone, "⌘Z did not move it back");
    assert!(redone, "⌘⇧Z did not move it again");
}
