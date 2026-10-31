use gpui_kit::{
    component::input::{Copy, Cut, Paste, Redo, Search, SelectAll, Undo},
    *,
};

use crate::theme::SelectTheme;

actions!(
    editor,
    [
        Save,
        CloseTab,
        NextTab,
        PrevTab,
        ToggleSidebar,
        Quit,
        OpenFolder,
        OpenRecent,
        CommandPalette,
        TogglePreview,
        ShowExplorer,
        ShowSearch,
        ShowGit,
        FileHistory,
        TogglePlay
    ]
);

// One list feeds both the menu bar and the palette, so neither can miss a command.
pub fn menus() -> Vec<(&'static str, Vec<(&'static str, Box<dyn Action>)>)> {
    vec![
        (
            "File",
            vec![
                ("Open Folder…", Box::new(OpenFolder)),
                ("Open Recent…", Box::new(OpenRecent)),
                ("Save", Box::new(Save)),
                ("Close Tab", Box::new(CloseTab)),
            ],
        ),
        (
            "Edit",
            vec![
                ("Undo", Box::new(Undo)),
                ("Redo", Box::new(Redo)),
                ("Cut", Box::new(Cut)),
                ("Copy", Box::new(Copy)),
                ("Paste", Box::new(Paste)),
                ("Select All", Box::new(SelectAll)),
                ("Find", Box::new(Search)),
            ],
        ),
        (
            "View",
            vec![
                ("Command Palette", Box::new(CommandPalette)),
                ("Explorer", Box::new(ShowExplorer)),
                ("Search in Folder", Box::new(ShowSearch)),
                ("Source Control", Box::new(ShowGit)),
                ("File History", Box::new(FileHistory)),
                ("Toggle Sidebar", Box::new(ToggleSidebar)),
                ("Next Tab", Box::new(NextTab)),
                ("Previous Tab", Box::new(PrevTab)),
                ("Toggle Preview", Box::new(TogglePreview)),
                ("Select Theme", Box::new(SelectTheme)),
            ],
        ),
    ]
}

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-s", Save, None),
        KeyBinding::new("cmd-w", CloseTab, None),
        KeyBinding::new("cmd-shift-]", NextTab, None),
        KeyBinding::new("cmd-shift-[", PrevTab, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-o", OpenFolder, None),
        KeyBinding::new("ctrl-r", OpenRecent, None),
        KeyBinding::new("cmd-shift-p", CommandPalette, None),
        KeyBinding::new("cmd-shift-v", TogglePreview, None),
        KeyBinding::new("cmd-shift-e", ShowExplorer, None),
        KeyBinding::new("cmd-shift-f", ShowSearch, None),
        KeyBinding::new("ctrl-shift-g", ShowGit, None),
        KeyBinding::new("space", TogglePlay, Some("Video")),
    ]);
    let app = Menu {
        name: "Editor".into(),
        items: vec![MenuItem::action("Quit Editor", Quit)],
        disabled: false,
    };
    let menus = menus().into_iter().map(|(name, items)| Menu {
        name: name.into(),
        items: items
            .into_iter()
            .map(|(name, action)| MenuItem::Action {
                name: name.into(),
                action,
                os_action: None,
                checked: false,
                disabled: false,
            })
            .collect(),
        disabled: false,
    });
    cx.set_menus(std::iter::once(app).chain(menus).collect::<Vec<_>>());
}
