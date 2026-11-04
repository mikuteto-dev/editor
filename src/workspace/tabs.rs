use std::path::{Path, PathBuf};

use gpui_kit::{
    component::{
        WindowExt,
        input::{EditorState, InputEvent, Position, Rope, TabSize},
    },
    *,
};

use super::Workspace;
use crate::{
    commands::{CloseTab, Quit, Save, TogglePlay, TogglePreview},
    tab::{Body, PreviewKind, PreviewPage, Tab, Text, is_image, language},

    video::{Video, is_video},
};

impl Workspace {
    pub(super) fn open(&mut self, path: PathBuf, pin: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.path == path && !matches!(t.body, Body::Preview(_))) {
            self.tabs[ix].pinned |= pin;
            return self.activate(ix, window, cx);
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned().into();
        let body = if is_image(&path) {
            Body::Image
        } else if is_video(&path) {
            match Video::new(&path) {
                Some(video) => Body::Video(video),
                None => return window.push_notification(format!("{}: cannot play", path.display()), cx),
            }
        } else {
            // shortcut: synchronous read stalls a frame on files over ~10MB; move to background_spawn if that matters.
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                Err(e) => return window.push_notification(format!("{}: {e}", path.display()), cx),
            };
            self.text(&path, text, language(&path), false, window, cx)
        };
        let ix = self.insert(Tab { path, name, body, pinned: pin }, window, cx);
        self.mark_changes(ix, cx);
        self.check_licenses(ix, cx);
    }

    /// Browsing replaces the clean preview tab instead of piling up new tabs.
    pub(super) fn insert(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let reusable = self.tabs.iter().position(|t| !t.pinned && !t.dirty());
        let ix = match reusable.filter(|_| !tab.pinned) {
            Some(ix) => {
                self.tabs[ix] = tab;
                ix
            }
            None => {
                self.tabs.push(tab);
                self.tabs.len() - 1
            }
        };
        self.activate(ix, window, cx);
        ix
    }

    pub(super) fn text(
        &mut self,
        path: &std::path::Path,
        text: String,
        language: &'static str,
        readonly: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Body {
        let previewable = PreviewKind::of(path);
        let saved = Rope::from_str(&text);
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .line_number(true)
                .indent_guides(true)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: false,
                })
                .default_value(text)
        });
        // Source files never execute or render on open; previews live in their own tabs.
        let preview = None;
        let licenses = crate::licenses::Lines::default();
        if !readonly {
            let hover = std::rc::Rc::new(super::licenses::LicenseHover(licenses.clone()));
            editor.update(cx, |editor, _| editor.lsp_mut().hover_provider = Some(hover));
        }
        let _subscription = cx.subscribe(&editor, |this, editor, e: &InputEvent, cx| {
            let InputEvent::Change = e else { return };
            let current = editor.read(cx).text().clone();
            let Some(tab) = this.tabs.iter_mut().find(|t| t.text().is_some_and(|t| t.editor == editor)) else {
                return;
            };
            if let Body::Text(text) = &mut tab.body {
                text.dirty = current != text.saved;
                // Editing keeps the tab, as in VS Code.
                tab.pinned |= text.dirty;
            }
            cx.notify();
        });
        Body::Text(Text {
            editor,
            language,
            saved,
            dirty: false,
            readonly,
            previewable,
            preview,
            git_marks: None,
            blame: Vec::new(),
            licenses,
            license_labels: None,
            _subscription,
        })
    }

    pub(super) fn text_mut(&mut self, editor: &Entity<EditorState>) -> Option<&mut Text> {
        self.tabs.iter_mut().find_map(|t| match &mut t.body {
            Body::Text(text) if text.editor == *editor => Some(text),
            _ => None,
        })
    }

    pub(super) fn open_at(&mut self, path: PathBuf, line: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.open(path, false, window, cx);
        if let Some(Tab { body: Body::Text(text), .. }) = self.tabs.get_mut(self.active) {
            text.preview = None;
            text.editor.update(cx, |editor, cx| {
                editor.set_cursor_position(Position::new(line, 0), window, cx)
            });
            self.activate(self.active, window, cx);
        }
    }

    pub(super) fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        if let Some(Tab { path, body: Body::Preview(page), .. }) = self.tabs.get_mut(ix)
            && !matches!(page.kind, PreviewKind::Tex)
        {
            let root = self.root.as_deref().unwrap_or_else(|| path.parent().unwrap_or(path));
            page.view = page.kind.render(&page.editor, path, root, cx);
        }
        // A hidden video keeps no sound or frames going.
        for (i, tab) in self.tabs.iter_mut().enumerate() {
            if let Body::Video(video) = &mut tab.body
                && i != ix
            {
                video.set_playing(false);
            }
        }
        let tab = self.tabs.get(ix);
        let path = tab.map(|t| t.path.clone());
        match tab.and_then(Tab::text).filter(|t| t.preview.is_none()) {
            Some(text) => text.editor.focus_handle(cx).focus(window, cx),
            // Keeps actions reachable when no editor holds focus.
            None => self.focus.focus(window, cx),
        }
        if let Some((tree, _)) = &self.tree {
            tree.update(cx, |tree, cx| tree.select(path, cx));
        }
        // Saved on every change, not at quit: quitting from the Dock or by logging out skips our Quit.
        self.save_session(window, cx);
        cx.notify();
    }

    /// Returns whether the tab is saved, so callers never drop unsaved text.
    fn write(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let tab = &mut self.tabs[ix];
        let Body::Text(text) = &mut tab.body else { return true };
        if text.readonly {
            return true;
        }
        let current = text.editor.read(cx).text().clone();
        match std::fs::write(&tab.path, current.to_string()) {
            Ok(()) => {
                text.saved = current;
                text.dirty = false;
                self.mark_changes(ix, cx);
                self.check_licenses(ix, cx);
                cx.notify();
                true
            }
            Err(e) => {
                window.push_notification(format!("{}: {e}", tab.path.display()), cx);
                false
            }
        }
    }

    pub(super) fn moved(&mut self, from: &Path, to: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        for tab in &mut self.tabs {
            if let Ok(rest) = tab.path.strip_prefix(from) {
                tab.path = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
                let name = tab.path.file_name().unwrap_or_default().to_string_lossy();
                tab.name = if matches!(tab.body, Body::Preview(_)) {
                    format!("{name} Preview").into()
                } else {
                    name.into_owned().into()
                };
                match &mut tab.body {
                    Body::Text(text) if matches!(text.preview, Some(crate::tab::Preview::Tex(_))) => text.preview = None,
                    Body::Preview(page) => {
                        let base = root.as_deref().unwrap_or_else(|| tab.path.parent().unwrap_or(&tab.path));
                        page.view = page.kind.render(&page.editor, &tab.path, base, cx);
                    }
                    _ => {}
                }
            }
        }
        self.activate(self.active, window, cx);
    }

    /// Closes what was moved to the Trash, except tabs holding unsaved edits.
    pub(super) fn deleted(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.retain(|t| !t.path.starts_with(path) || t.dirty());
        self.activate(self.active.min(self.tabs.len().saturating_sub(1)), window, cx);
    }

    fn remove(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.tabs.remove(ix);
        let active = if self.active > ix { self.active - 1 } else { self.active };
        self.activate(active.min(self.tabs.len().saturating_sub(1)), window, cx);
    }

    pub(super) fn confirm_discard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if !self.tabs.iter().any(Tab::dirty) {
            return then(self, window, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            "Save changes before closing?",
            None,
            &["Save All", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await.ok()?;
            this.update_in(cx, |this, window, cx| {
                let proceed = match choice {
                    0 => (0..this.tabs.len())
                        .filter(|&ix| this.tabs[ix].dirty())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .all(|ix| this.write(ix, window, cx)),
                    1 => true,
                    _ => false,
                };
                if proceed {
                    then(this, window, cx);
                }
            })
            .ok()
        })
        .detach();
    }

    pub(super) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.active < self.tabs.len() {
            self.write(self.active, window, cx);
        }
    }

    pub(super) fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close(self.active, window, cx);
    }

    pub(super) fn close(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if !tab.dirty() {
            return self.remove(ix, window, cx);
        }
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to {}?", tab.name),
            None,
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let choice = answer.await.ok()?;
            this.update_in(cx, |this, window, cx| match choice {
                0 if this.write(ix, window, cx) => this.remove(ix, window, cx),
                1 => this.remove(ix, window, cx),
                _ => {}
            })
            .ok()
        })
        .detach();
    }

    pub(super) fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.confirm_discard(window, cx, |this, window, cx| {
            this.save_session(window, cx);
            cx.quit()
        });
    }

    pub(super) fn cycle(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.tabs.len() as isize;
        if len > 0 {
            self.activate((self.active as isize + step).rem_euclid(len) as usize, window, cx);
        }
    }

    pub(super) fn toggle_play(&mut self, _: &TogglePlay, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(Tab { body: Body::Video(video), .. }) = self.tabs.get_mut(self.active) {
            video.toggle();
            cx.notify();
        }
    }

    pub(super) fn toggle_preview(&mut self, _: &TogglePreview, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        if matches!(tab.body, Body::Preview(_)) {
            let path = tab.path.clone();
            if let Some(ix) = self.tabs.iter().position(|tab| tab.path == path && matches!(tab.body, Body::Text(_))) {
                self.activate(ix, window, cx);
            } else {
                self.open(path.clone(), true, window, cx);
                let Some(editor) = self.tabs.get(self.active).and_then(Tab::text).map(|text| text.editor.clone()) else {
                    return;
                };
                let root = self.root.as_deref().unwrap_or_else(|| path.parent().unwrap_or(&path));
                if let Some(Body::Preview(page)) = self
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.path == path && matches!(tab.body, Body::Preview(_)))
                    .map(|tab| &mut tab.body)
                {
                    page.editor = editor;
                    page.view = page.kind.render(&page.editor, &path, root, cx);
                }
            }
            return;
        }
        let Body::Text(text) = &tab.body else { return };
        let Some(kind) = text.previewable else { return };
        let path = tab.path.clone();
        if let Some(ix) = self.tabs.iter().position(|tab| tab.path == path && matches!(tab.body, Body::Preview(_))) {
            self.activate(ix, window, cx);
            return;
        }
        let editor = text.editor.clone();
        let name = format!("{} Preview", tab.name).into();
        let root = self.root.as_deref().unwrap_or_else(|| path.parent().unwrap_or(&path));
        let view = kind.render(&editor, &path, root, cx);
        self.tabs[self.active].pinned = true;
        self.insert(
            Tab { path, name, body: Body::Preview(PreviewPage { editor, kind, view }), pinned: true },
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use gpui::AppContext as _;

    use super::{Body, TogglePreview, Workspace};

    #[gpui::test]
    fn preview_opens_in_a_separate_tab(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_kit::init);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.md");
        std::fs::write(&path, "# Page").unwrap();
        let (workspace, cx) = cx.add_window_view(|window, cx| Workspace::new(None, window, cx));
        cx.update(|window, cx| workspace.update(cx, |workspace, cx| workspace.open(path.clone(), true, window, cx)));
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| workspace.toggle_preview(&TogglePreview, window, cx))
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.len(), 2);
            assert!(matches!(workspace.tabs[0].body, Body::Text(_)));
            assert!(matches!(workspace.tabs[1].body, Body::Preview(_)));
            assert_eq!(workspace.active, 1);
        });
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| workspace.toggle_preview(&TogglePreview, window, cx))
        });
        assert_eq!(workspace.read_with(cx, |workspace, _| workspace.active), 0);
        cx.update(|window, cx| {
            workspace.update(cx, |workspace, cx| workspace.toggle_preview(&TogglePreview, window, cx))
        });
        workspace.read_with(cx, |workspace, _| {
            assert_eq!(workspace.tabs.len(), 2);
            assert_eq!(workspace.active, 1);
        });
    }
}
