use std::{cell::RefCell, rc::Rc};

use gpui_kit::{
    component::{ActiveTheme, Theme, ThemeRegistry, WindowExt, command::CommandItem},
    *,
};

use crate::{config_dir, palette};

actions!(editor, [SelectTheme]);

const BUNDLED: &[&str] = &[
    include_str!("../assets/themes/tokyonight.json"),
    include_str!("../assets/themes/catppuccin.json"),
    include_str!("../assets/themes/everforest.json"),
    include_str!("../assets/themes/solarized.json"),
    include_str!("../assets/themes/gruvbox.json"),
];

const DEFAULT: &str = "Catppuccin Macchiato";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-k cmd-t", SelectTheme, None)]);
    load(cx);
    let name = std::fs::read_to_string(config_dir().join("theme")).unwrap_or_default();
    if !apply(name.trim(), cx) {
        apply(DEFAULT, cx);
    }
}

// shortcut: a theme already registered keeps its first definition, so edits to a user theme need a restart; add a file watcher if live editing is wanted.
fn load(cx: &mut App) {
    let user = std::fs::read_dir(config_dir().join("themes"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok());
    let registry = ThemeRegistry::global_mut(cx);
    for json in BUNDLED.iter().map(|s| s.to_string()).chain(user) {
        if let Err(e) = registry.load_themes_from_str(&json) {
            eprintln!("theme: {e}");
        }
    }
}

fn apply(name: &str, cx: &mut App) -> bool {
    let Some(config) = ThemeRegistry::global(cx).themes().get(name).cloned() else {
        return false;
    };
    Theme::update(cx, |theme| theme.apply_config(&config));
    true
}

pub fn open_picker(window: &mut Window, cx: &mut App) {
    load(cx);
    let mut names: Vec<SharedString> = ThemeRegistry::global(cx)
        .themes()
        .values()
        .filter(|t| t.mode.is_dark())
        .map(|t| t.name.clone())
        .collect();
    names.sort();
    let current = cx.theme().theme_name().clone();
    let items = names
        .iter()
        .map(|n| CommandItem::new().label(n.clone()).checked(*n == current))
        .collect();
    let selected = names.iter().position(|n| *n == current);
    // Holds the theme to restore until the user confirms another one.
    let restore = Rc::new(RefCell::new(Some(current)));
    let names = Rc::new(names);
    let (select, confirm, close) = (names.clone(), names, restore.clone());
    palette::open(
        items,
        selected,
        move |ix, _, cx| {
            apply(&select[ix], cx);
        },
        move |ix, window, cx| {
            restore.borrow_mut().take();
            let saved = std::fs::create_dir_all(config_dir())
                .and_then(|()| std::fs::write(config_dir().join("theme"), confirm[ix].as_bytes()));
            if let Err(e) = saved {
                window.push_notification(format!("theme: {e}"), cx);
            }
        },
        move |_, cx| {
            if let Some(name) = close.borrow_mut().take() {
                apply(&name, cx);
            }
        },
        window,
        cx,
    );
}
