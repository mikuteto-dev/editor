use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui_kit::{
    component::{ActiveTheme, WindowExt, h_flex},
    *,
};

use super::{FileTree, TreeEvent, undo::Op};
use crate::{files, icons};

/// How long a drag rests on a closed folder before it starts to open: Zed's and VS Code's wait.
const SPRING_DELAY: Duration = Duration::from_millis(500);
/// The folder then blinks twice, as Finder's spring-loaded folders do, and opens.
pub(super) const BLINK: Duration = Duration::from_millis(300);

#[derive(Default)]
pub(super) struct Spring {
    waiting: Option<(PathBuf, Task<()>)>,
    /// The folder blinking before it opens, numbered so each blink plays from the start.
    pub blinking: Option<(PathBuf, usize)>,
    blinks: usize,
}

#[derive(Clone)]
pub(super) struct Dragged {
    pub path: PathBuf,
    pub dir: bool,
}

pub(super) struct Ghost(Dragged);

impl Render for Ghost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let name = self.0.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let icon = if self.0.dir { icons::folder(&name, false) } else { icons::file(&name) };
        h_flex()
            .gap_1p5()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .text_sm()
            .text_color(theme.popover_foreground)
            .child(img(icon).size_4())
            .child(name)
    }
}

impl FileTree {
    pub(super) fn ghost(dragged: &Dragged, cx: &mut App) -> Entity<Ghost> {
        cx.new(|_| Ghost(dragged.clone()))
    }

    pub(super) fn drop_dir(&self, path: &Path, dir: bool) -> PathBuf {
        if dir { path.to_path_buf() } else { path.parent().map_or(self.root.clone(), Path::to_path_buf) }
    }

    /// A trackpad's small movements within the row do not start the wait over, as in Zed.
    pub(super) fn drag_over_folder(&mut self, path: PathBuf, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if self.spring.waiting.as_ref().is_some_and(|(p, _)| *p == path) {
            return;
        }
        let target = path.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            let path = target;
            let still_on = |window: &Window, cx: &App| bounds.contains(&window.mouse_position()) && cx.has_active_drag();
            cx.background_executor().timer(SPRING_DELAY).await;
            let blinking = this.update_in(cx, |this, window, cx| {
                if !still_on(window, cx) {
                    this.spring.waiting = None;
                    return false;
                }
                this.spring.blinks += 1;
                this.spring.blinking = Some((path.clone(), this.spring.blinks));
                cx.notify();
                true
            });
            if !blinking.unwrap_or(false) {
                return;
            }
            cx.background_executor().timer(BLINK).await;
            this.update_in(cx, |this, window, cx| {
                this.spring.waiting = None;
                this.spring.blinking = None;
                let closed = this.rows.iter().position(|r| r.path == path && r.dir && !r.open);
                if let Some(ix) = closed.filter(|_| still_on(window, cx)) {
                    this.click(ix, false, cx);
                }
                cx.notify();
            })
            .ok();
        });
        self.spring.waiting = Some((path, task));
    }

    pub(super) fn drop_entry(&mut self, dragged: &Dragged, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        // ⌘Z right after a drop should reach the tree, not the editor that had focus.
        self.focus.focus(window, cx);
        match files::move_into(&dragged.path, &dir) {
            Ok(to) if to == dragged.path => {}
            Ok(to) => {
                let from = dragged.path.clone();
                let mut dirs = vec![dir];
                dirs.extend(from.parent().map(Path::to_path_buf));
                self.refresh(&dirs, cx);
                self.selected = Some(to.clone());
                self.history.record(vec![Op::Move { from: from.clone(), to: to.clone() }]);
                cx.emit(TreeEvent::Renamed { from, to });
            }
            Err(e) => window.push_notification(e, cx),
        }
    }

    pub(super) fn drop_external(&mut self, paths: &ExternalPaths, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let mut copied = Vec::new();
        for path in paths.paths() {
            match files::copy_into(path, &dir) {
                Ok(to) => copied.push(Op::Create(to)),
                Err(e) => window.push_notification(e, cx),
            }
        }
        self.history.record(copied);
        self.refresh(&[dir], cx);
    }
}
