use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui_kit::{component::command::CommandItem, *};

use super::Workspace;
use crate::{
    commands::{CommandPalette, OpenFolder, OpenRecent, menus},
    palette, recent,
};

impl Workspace {
    pub(super) fn open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let dir = paths.await.ok()?.ok()??.into_iter().next()?;
            this.update_in(cx, |this, window, cx| this.switch(dir, window, cx)).ok()
        })
        .detach();
    }

    pub(super) fn open_recent(&mut self, _: &OpenRecent, window: &mut Window, cx: &mut Context<Self>) {
        let dirs: Vec<PathBuf> = recent::load()
            .into_iter()
            .filter(|d| self.root.as_ref() != Some(d) && d.is_dir())
            .collect();
        if dirs.is_empty() {
            return self.open_folder(&OpenFolder, window, cx);
        }
        let items = dirs
            .iter()
            .map(|d| {
                let name = d.file_name().unwrap_or_default().to_string_lossy();
                CommandItem::new().label(format!("{name}    {}", tilde(d)))
            })
            .collect();
        let this = cx.entity().downgrade();
        palette::open(
            items,
            Some(0),
            |_, _, _| {},
            move |ix, window, cx| {
                this.update(cx, |this, cx| this.switch(dirs[ix].clone(), window, cx)).ok();
            },
            |_, _| {},
            window,
            cx,
        );
    }

    pub(super) fn command_palette(&mut self, _: &CommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        let commands: Rc<Vec<_>> = Rc::new(menus().into_iter().flat_map(|(_, items)| items).collect());
        let items = commands
            .iter()
            .map(|(label, action)| CommandItem::new().label(*label).action(action.boxed_clone()))
            .collect();
        let focus = window.focused(cx);
        palette::open(
            items,
            None,
            |_, _, _| {},
            move |ix, window, cx| {
                // The palette dispatches from its own focus, outside this view;
                // run the command again from where the user was.
                let (action, focus) = (commands[ix].1.boxed_clone(), focus.clone());
                window.defer(cx, move |window, cx| {
                    if let Some(focus) = focus {
                        focus.focus(window, cx);
                    }
                    window.dispatch_action(action, cx);
                });
            },
            |_, _| {},
            window,
            cx,
        );
    }
}

fn tilde(path: &Path) -> String {
    match std::env::home_dir().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}
