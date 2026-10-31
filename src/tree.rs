use std::{
    collections::HashMap,
    ops::Range,
    path::{Path, PathBuf},
};

use gpui_kit::{
    component::{ActiveTheme, Icon, IconName, h_flex, v_flex},
    prelude::FluentBuilder,
    *,
};

use crate::{git::Change, git_panel::status_color, icons};

actions!(tree, [Up, Down, Expand, Collapse, Enter]);

pub fn init(cx: &mut App) {
    let context = Some("FileTree");
    cx.bind_keys([
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("right", Expand, context),
        KeyBinding::new("left", Collapse, context),
        KeyBinding::new("enter", Enter, context),
    ]);
}

pub struct FileTree {
    root: PathBuf,
    rows: Vec<Row>,
    selected: Option<PathBuf>,
    /// Git status by path; folders carry the status of a change inside them.
    status: HashMap<PathBuf, char>,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
}

struct Row {
    path: PathBuf,
    name: SharedString,
    depth: usize,
    dir: bool,
    open: bool,
}

pub struct OpenFile {
    pub path: PathBuf,
    pub pin: bool,
}

impl EventEmitter<OpenFile> for FileTree {}

impl FileTree {
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut tree = Self {
            root: root.clone(),
            rows: Vec::new(),
            selected: None,
            status: HashMap::new(),
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
        };
        tree.load(root, None, 0, cx);
        tree
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    fn selected_ix(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.rows.iter().position(|r| &r.path == selected)
    }

    fn move_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(ix) {
            self.selected = Some(row.path.clone());
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
            cx.notify();
        }
    }

    fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        let ix = match self.selected_ix() {
            Some(ix) => ix.saturating_add_signed(delta).min(self.rows.len().saturating_sub(1)),
            None => 0,
        };
        self.move_to(ix, cx);
    }

    fn fold(&mut self, open: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.selected_ix() else { return };
        let row = &self.rows[ix];
        if row.dir && row.open != open {
            return self.click(ix, false, cx);
        }
        if !open && let Some(parent) = self.rows[..ix].iter().rposition(|r| r.depth < row.depth) {
            self.move_to(parent, cx);
        }
    }

    pub fn set_status(&mut self, changes: &[Change], cx: &mut Context<Self>) {
        self.status = status_map(&self.root, changes);
        cx.notify();
    }

    pub fn select(&mut self, path: Option<PathBuf>, cx: &mut Context<Self>) {
        self.selected = path;
        cx.notify();
    }

    fn load(&mut self, dir: PathBuf, parent: Option<PathBuf>, depth: usize, cx: &mut Context<Self>) {
        let entries = cx.background_spawn(async move { read_dir(&dir, depth) });
        cx.spawn(async move |this, cx| {
            let entries = entries.await;
            this.update(cx, |this, cx| {
                let at = match &parent {
                    None if this.rows.is_empty() => 0,
                    None => return,
                    Some(parent) => {
                        let Some(ix) = this.rows.iter().position(|r| &r.path == parent) else {
                            return;
                        };
                        // Collapsed while loading, or loaded twice by a quick re-open.
                        let loaded = this.rows.get(ix + 1).is_some_and(|r| r.depth > depth - 1);
                        if !this.rows[ix].open || loaded {
                            return;
                        }
                        ix + 1
                    }
                };
                this.rows.splice(at..at, entries);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn click(&mut self, ix: usize, pin: bool, cx: &mut Context<Self>) {
        let row = &mut self.rows[ix];
        if !row.dir {
            let path = row.path.clone();
            self.selected = Some(path.clone());
            cx.emit(OpenFile { path, pin });
            cx.notify();
            return;
        }
        row.open = !row.open;
        if row.open {
            let (path, depth) = (row.path.clone(), row.depth + 1);
            self.load(path.clone(), Some(path), depth, cx);
        } else {
            let end = self.subtree_end(ix);
            self.rows.drain(ix + 1..end);
        }
        cx.notify();
    }

    fn subtree_end(&self, ix: usize) -> usize {
        let depth = self.rows[ix].depth;
        self.rows[ix + 1..]
            .iter()
            .position(|r| r.depth <= depth)
            .map_or(self.rows.len(), |n| ix + 1 + n)
    }

    // shortcut: reads on the main thread; fine for directory listings, move to background_spawn if huge dirs stutter.
    pub fn refresh(&mut self, dirs: &[PathBuf], cx: &mut Context<Self>) {
        for dir in dirs {
            let span = if *dir == self.root {
                Some((0, self.rows.len(), 0))
            } else {
                self.rows
                    .iter()
                    .position(|r| r.path == *dir && r.open)
                    .map(|ix| (ix + 1, self.subtree_end(ix), self.rows[ix].depth + 1))
            };
            let Some((start, end, depth)) = span else { continue };
            let old: Vec<Row> = self.rows.drain(start..end).collect();
            let rows = merge(old, read_dir(dir, depth), depth);
            self.rows.splice(start..start, rows);
        }
        cx.notify();
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        let hover = theme.sidebar_accent;
        range
            .map(|ix| {
                let row = &self.rows[ix];
                let selected = self.selected.as_ref() == Some(&row.path);
                let chevron = row.dir.then(|| {
                    Icon::new(if row.open {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size_3p5()
                    .text_color(theme.muted_foreground)
                });
                let icon = if row.dir {
                    icons::folder()
                } else {
                    icons::file(&row.name)
                };
                let status = self.status.get(&row.path).copied();
                let color = status.map(|s| status_color(s, theme));
                let mark = status.map(|s| {
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(status_color(s, theme))
                        .child(if row.dir { "•".to_string() } else { s.to_string() })
                });
                h_flex()
                    .id(ix)
                    .h(px(24.))
                    .w_full()
                    .pl(px(6. + 12. * row.depth as f32))
                    .pr_2()
                    .gap_1p5()
                    .rounded_sm()
                    .when(selected, |el| el.bg(theme.sidebar_accent))
                    .hover(move |el| el.bg(hover))
                    .child(div().w_3p5().flex_none().children(chevron))
                    .child(
                        h_flex()
                            .flex_1()
                            .gap_1p5()
                            .min_w_0()
                            // Hidden entries fade back, as in Finder.
                            .when(row.name.starts_with('.'), |el| el.opacity(0.5))
                            .child(img(icon).size_4().flex_none())
                            .child(div().truncate().when_some(color, |el, c| el.text_color(c)).child(row.name.clone())),
                    )
                    .children(mark)
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        this.click(ix, event.click_count() > 1, cx)
                    }))
            })
            .collect()
    }
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.root.file_name().unwrap_or(self.root.as_os_str()).to_string_lossy();
        v_flex()
            .size_full()
            .bg(cx.theme().sidebar)
            .key_context("FileTree")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Up, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Down, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &Expand, _, cx| this.fold(true, cx)))
            .on_action(cx.listener(|this, _: &Collapse, _, cx| this.fold(false, cx)))
            .on_action(cx.listener(|this, _: &Enter, _, cx| {
                if let Some(ix) = this.selected_ix() {
                    this.click(ix, true, cx);
                }
            }))
            .child(
                h_flex()
                    .h(px(28.))
                    .flex_none()
                    .px_3()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(name.to_uppercase()),
            )
            .child(
                uniform_list(
                    "tree",
                    self.rows.len(),
                    cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0()
                .w_full()
                .px_1()
                .pb_1()
                .text_sm()
                .text_color(cx.theme().sidebar_foreground),
            )
    }
}

fn status_map(root: &Path, changes: &[Change]) -> HashMap<PathBuf, char> {
    let mut map = HashMap::new();
    for change in changes {
        map.insert(change.path.clone(), change.status);
        for dir in change.path.ancestors().skip(1).take_while(|d| d.starts_with(root) && *d != root) {
            // A folder with mixed changes reads as modified.
            let status = map.entry(dir.to_path_buf()).or_insert(change.status);
            if *status != change.status {
                *status = 'M';
            }
        }
    }
    map
}

#[std::prelude::rust_2024::test]
fn folders_take_the_status_inside() {
    let change = |path: &str, status| Change { path: path.into(), status, staged: false, unstaged: true };
    let map = status_map(Path::new("/r"), &[change("/r/a/new.rs", 'U'), change("/r/b/c/x.rs", 'U'), change("/r/b/y.rs", 'M')]);
    assert_eq!(map[Path::new("/r/a")], 'U');
    assert_eq!(map[Path::new("/r/b/c")], 'U');
    assert_eq!(map[Path::new("/r/b")], 'M');
    assert!(!map.contains_key(Path::new("/r")));
}

// Keeps folders the user expanded open across a re-read.
fn merge(old: Vec<Row>, fresh: Vec<Row>, depth: usize) -> Vec<Row> {
    let mut kept: HashMap<PathBuf, Vec<Row>> = HashMap::new();
    let mut current = None;
    for row in old {
        if row.depth == depth {
            current = Some(row.path.clone());
        }
        if let Some(path) = &current {
            kept.entry(path.clone()).or_default().push(row);
        }
    }
    fresh
        .into_iter()
        .flat_map(|row| kept.remove(&row.path).unwrap_or_else(|| vec![row]))
        .collect()
}

#[std::prelude::rust_2024::test]
fn merge_keeps_open_subtrees() {
    let row = |path: &str, depth, open| Row {
        path: path.into(),
        name: path.into(),
        depth,
        dir: true,
        open,
    };
    let old = vec![row("a", 0, true), row("a/x", 1, false), row("b", 0, false), row("gone", 0, false)];
    let fresh = vec![row("a", 0, false), row("b", 0, false), row("new", 0, false)];
    let names: Vec<_> = merge(old, fresh, 0).iter().map(|r| (r.name.to_string(), r.open)).collect();
    let expect = [("a", true), ("a/x", false), ("b", false), ("new", false)];
    assert_eq!(names, expect.map(|(n, o)| (n.to_string(), o)));
}

fn read_dir(dir: &Path, depth: usize) -> Vec<Row> {
    let mut rows: Vec<Row> = ignore::WalkBuilder::new(dir)
        .max_depth(Some(1))
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git" && e.file_name() != ".DS_Store")
        .build()
        .skip(1)
        .filter_map(Result::ok)
        .map(|e| {
            let dir = e
                .file_type()
                .is_some_and(|t| t.is_dir() || t.is_symlink() && e.path().is_dir());
            Row {
                name: e.file_name().to_string_lossy().into_owned().into(),
                path: e.into_path(),
                depth,
                dir,
                open: false,
            }
        })
        .collect();
    rows.sort_by_cached_key(|r| (!r.dir, r.name.to_lowercase()));
    rows
}

#[std::prelude::rust_2024::test]
fn reads_one_level_sorted_and_gitignored() {
    let dir = std::env::temp_dir().join(format!("editor-tree-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::create_dir_all(dir.join("src/inner")).unwrap();
    for f in ["b.rs", "A.md", "skip.log", ".gitignore"] {
        std::fs::write(dir.join(f), "*.log\n").unwrap();
    }
    let names: Vec<_> = read_dir(&dir, 0).iter().map(|r| r.name.to_string()).collect();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(names, ["src", ".gitignore", "A.md", "b.rs"]);
}
