use std::path::{Path, PathBuf};

use gpui_kit::{
    component::{
        Icon, Sizable, WindowExt,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        menu::{PopupMenu, PopupMenuItem},
    },
    *,
};

use super::{FileTree, TreeEvent, undo::Op};
use crate::files;

const FILE_PLUS: &[u8] = include_bytes!("../../assets/ui/file-plus.svg");
const FOLDER_PLUS: &[u8] = include_bytes!("../../assets/ui/folder-plus.svg");
const REFRESH: &[u8] = include_bytes!("../../assets/ui/refresh-cw.svg");
const COLLAPSE: &[u8] = include_bytes!("../../assets/ui/chevrons-down-up.svg");
const RENAME: &[u8] = include_bytes!("../../assets/ui/pencil.svg");
const TRASH: &[u8] = include_bytes!("../../assets/ui/trash.svg");
const COPY: &[u8] = include_bytes!("../../assets/ui/copy.svg");
const FINDER: &[u8] = include_bytes!("../../assets/ui/folder-search.svg");

pub(super) enum Kind {
    File,
    Folder,
    Rename(PathBuf),
}

pub(super) struct Editing {
    pub kind: Kind,
    pub dir: PathBuf,
    pub input: Entity<InputState>,
    _events: Subscription,
}

pub(super) enum Slot {
    Row(usize),
    Input(usize),
}

impl FileTree {
    fn target_dir(&self) -> PathBuf {
        let selected = self.selected_ix().map(|ix| &self.rows[ix]);
        match selected {
            Some(row) if row.dir => row.path.clone(),
            Some(row) => row.path.parent().map_or(self.root.clone(), Path::to_path_buf),
            None => self.root.clone(),
        }
    }

    fn input_at(&self) -> Option<(usize, usize)> {
        let editing = self.editing.as_ref()?;
        if matches!(editing.kind, Kind::Rename(_)) {
            return None;
        }
        if editing.dir == self.root {
            return Some((0, 0));
        }
        let ix = self.rows.iter().position(|r| r.path == editing.dir)?;
        Some((ix + 1, self.rows[ix].depth + 1))
    }

    pub(super) fn list_len(&self) -> usize {
        self.rows.len() + self.input_at().is_some() as usize
    }

    pub(super) fn slot(&self, ix: usize) -> Slot {
        match self.input_at() {
            Some((at, depth)) if ix == at => Slot::Input(depth),
            Some((at, _)) if ix > at => Slot::Row(ix - 1),
            _ => Slot::Row(ix),
        }
    }

    pub(super) fn renaming(&self, path: &Path) -> bool {
        self.editing.as_ref().is_some_and(|e| matches!(&e.kind, Kind::Rename(p) if p == path))
    }

    pub(super) fn begin_new(&mut self, dir: PathBuf, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.rows.iter().position(|r| r.path == dir && r.dir && !r.open) {
            self.click(ix, false, cx);
        }
        let kind = if folder { Kind::Folder } else { Kind::File };
        self.begin(kind, dir, "", window, cx);
    }

    pub(super) fn begin_rename(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let dir = path.parent().map_or(self.root.clone(), Path::to_path_buf);
        self.begin(Kind::Rename(path), dir, &name, window, cx);
    }

    fn begin(&mut self, kind: Kind, dir: PathBuf, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match kind {
            Kind::File => "File name",
            Kind::Folder => "Folder name",
            Kind::Rename(_) => "",
        };
        let value = value.to_string();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value));
        let _events = cx.subscribe_in(&input, window, |this, _, e: &InputEvent, window, cx| match e {
            InputEvent::PressEnter { .. } => this.commit(window, cx),
            // Clicking away keeps what was typed, as in VS Code; an empty name means never mind.
            InputEvent::Blur => this.commit(window, cx),
            _ => {}
        });
        // After the menu that started this has closed and handed focus back.
        let focus = input.read(cx).focus_handle(cx);
        window.defer(cx, move |window, cx| focus.focus(window, cx));
        self.editing = Some(Editing { kind, dir, input, _events });
        cx.notify();
    }

    pub(super) fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editing) = self.editing.take() else { return };
        let name = editing.input.read(cx).value().to_string();
        if name.trim().is_empty() {
            self.focus.focus(window, cx);
            return cx.notify();
        }
        let made = match &editing.kind {
            Kind::File => files::create_file(&editing.dir, &name),
            Kind::Folder => files::create_folder(&editing.dir, &name),
            Kind::Rename(from) => files::rename(from, &name),
        };
        let path = match made {
            Ok(path) => path,
            Err(e) => {
                window.push_notification(e, cx);
                // Keep the input so the name can be fixed.
                self.editing = Some(editing);
                return cx.notify();
            }
        };
        self.refresh(&[editing.dir.clone()], cx);
        self.selected = Some(path.clone());
        self.focus.focus(window, cx);
        match editing.kind {
            Kind::File => {
                self.history.record(vec![Op::Create(path.clone())]);
                cx.emit(TreeEvent::Open { path, pin: true })
            }
            Kind::Folder => self.history.record(vec![Op::Create(path)]),
            // A rename that changed nothing leaves nothing to undo.
            Kind::Rename(from) if from == path => {}
            Kind::Rename(from) => {
                self.history.record(vec![Op::Move { from: from.clone(), to: path.clone() }]);
                cx.emit(TreeEvent::Renamed { from, to: path })
            }
        }
        cx.notify();
    }

    pub(super) fn delete(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Move “{name}” to the Trash?"),
            Some("⌘Z puts it back."),
            &["Move to Trash", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await.ok()? != 0 {
                return None;
            }
            this.update_in(cx, |this, window, cx| match files::trash(&path) {
                Ok(trashed) => {
                    if let Some(parent) = path.parent() {
                        this.refresh(&[parent.to_path_buf()], cx);
                    }
                    this.history.record(vec![Op::Move { from: path.clone(), to: trashed }]);
                    this.focus.focus(window, cx);
                    cx.emit(TreeEvent::Deleted(path));
                }
                Err(e) => window.push_notification(e, cx),
            })
            .ok()
        })
        .detach();
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.rows.retain(|r| r.depth == 0);
        for row in &mut self.rows {
            row.open = false;
        }
        cx.notify();
    }

    /// Reads the root and every open folder again, for changes the watcher could not see.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let mut dirs = vec![self.root.clone()];
        dirs.extend(self.rows.iter().filter(|r| r.dir && r.open).map(|r| r.path.clone()));
        self.refresh(&dirs, cx);
    }

    pub(super) fn render_input(&self, depth: usize, cx: &App) -> AnyElement {
        let Some(editing) = &self.editing else { return div().into_any_element() };
        let icon = match editing.kind {
            Kind::Folder => crate::icons::folder("", false),
            _ => crate::icons::file(&editing.input.read(cx).value()),
        };
        h_flex()
            .id("new-entry")
            .h(px(24.))
            .w_full()
            .pl(px(6. + 12. * depth as f32))
            .pr_1()
            .gap_1p5()
            .child(div().w_3p5().flex_none())
            .child(img(icon).size_4().flex_none())
            .child(self.render_field(cx))
            .into_any_element()
    }

    pub(super) fn render_field(&self, _: &App) -> AnyElement {
        match &self.editing {
            Some(editing) => div().flex_1().min_w_0().child(Input::new(&editing.input).xsmall()).into_any_element(),
            None => div().into_any_element(),
        }
    }

    pub(super) fn render_actions(&self, cx: &mut Context<Self>) -> Div {
        let button = |id: &'static str, svg: &'static [u8], tip: &'static str| {
            Button::new(id).icon(Icon::default().data(svg)).ghost().xsmall().tooltip(tip)
        };
        h_flex()
            .gap_0p5()
            // Shown while the pointer is over the tree, as in VS Code.
            .invisible()
            .group_hover("explorer", |style| style.visible())
            .child(button("new-file", FILE_PLUS, "New File…").on_click(cx.listener(|this, _, window, cx| {
                let dir = this.target_dir();
                this.begin_new(dir, false, window, cx)
            })))
            .child(button("new-folder", FOLDER_PLUS, "New Folder…").on_click(cx.listener(|this, _, window, cx| {
                let dir = this.target_dir();
                this.begin_new(dir, true, window, cx)
            })))
            .child(button("reload", REFRESH, "Refresh").on_click(cx.listener(|this, _, _, cx| this.reload(cx))))
            .child(button("collapse", COLLAPSE, "Collapse All").on_click(cx.listener(|this, _, _, cx| this.collapse_all(cx))))
    }
}

pub(super) fn menu(menu: PopupMenu, tree: WeakEntity<FileTree>, path: PathBuf, dir: bool, root: PathBuf) -> PopupMenu {
    let folder = if dir { path.clone() } else { path.parent().map_or(root.clone(), Path::to_path_buf) };
    let item = |label: &'static str, svg: &'static [u8], run: Box<dyn Fn(&mut FileTree, &mut Window, &mut Context<FileTree>)>| {
        let tree = tree.clone();
        PopupMenuItem::new(label).icon(Icon::default().data(svg)).on_click(move |_, window, cx| {
            tree.update(cx, |tree, cx| run(tree, window, cx)).ok();
        })
    };
    let relative = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().into_owned();
    let absolute = path.to_string_lossy().into_owned();
    // The root itself is not renamed or trashed from inside its own tree.
    let entry = path != root;
    let (f1, f2, p1, p2, p3) = (folder.clone(), folder, path.clone(), path.clone(), path);
    let menu = menu
        .item(item("New File…", FILE_PLUS, Box::new(move |t, w, cx| t.begin_new(f1.clone(), false, w, cx))))
        .item(item("New Folder…", FOLDER_PLUS, Box::new(move |t, w, cx| t.begin_new(f2.clone(), true, w, cx))))
        .separator()
        .item(item("Reveal in Finder", FINDER, Box::new(move |_, _, _| files::reveal(&p1))))
        .item(item("Copy Path", COPY, Box::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(absolute.clone())))));
    if !entry {
        return menu;
    }
    menu.item(item("Copy Relative Path", COPY, Box::new(move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(relative.clone())))))
        .separator()
        .item(item("Rename…", RENAME, Box::new(move |t, w, cx| t.begin_rename(p2.clone(), w, cx))))
        .item(item("Move to Trash", TRASH, Box::new(move |t, w, cx| t.delete(p3.clone(), w, cx))))
}
