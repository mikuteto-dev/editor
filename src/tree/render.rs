use std::ops::Range;

use gpui_kit::{
    component::{
        ActiveTheme, Icon, IconName, h_flex,
        input::{Redo, Undo},
        menu::ContextMenuExt,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::{
    CancelEdit, Collapse, Down, Enter, Expand, FileTree, Up,
    drag::{BLINK, Dragged},
    edit,
    edit::Slot,
};
use crate::{git_panel::status_color, icons};

impl FileTree {
    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let theme = &theme;
        let hover = theme.sidebar_accent;
        let tree = cx.entity().downgrade();
        range
            .map(|ix| {
                let ix = match self.slot(ix) {
                    Slot::Row(ix) => ix,
                    Slot::Input(depth) => return self.render_input(depth, cx),
                };
                let row = &self.rows[ix];
                let renaming = self.renaming(&row.path);
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
                    icons::folder(&row.name, row.open)
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
                let name = match renaming {
                    true => self.render_field(cx),
                    false => div().truncate().when_some(color, |el, c| el.text_color(c)).child(row.name.clone()).into_any_element(),
                };
                let (path, dir, root, tree) = (row.path.clone(), row.dir, self.root.clone(), tree.clone());
                let (into, into_external) = (self.drop_dir(&path, dir), self.drop_dir(&path, dir));
                let drop_bg = theme.drop_target;
                let closed_folder = (dir && !row.open).then(|| path.clone());
                let spring = closed_folder.clone();
                let blink = self.spring.blinking.as_ref().filter(|(p, _)| *p == path).map(|(_, n)| *n);
                let flash = theme.primary.opacity(0.35);
                h_flex()
                    .id(("row", ix))
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
                            .when(row.name.starts_with('.') && !renaming, |el| el.opacity(0.5))
                            .child(img(icon).size_4().flex_none())
                            .child(name),
                    )
                    .children(mark)
                    // Two quick flashes before a drag opens the folder, as in Finder.
                    .when_some(blink, |el, n| {
                        el.relative().child(
                            div().absolute().inset_0().rounded_sm().with_animation(
                                ("blink", n),
                                Animation::new(BLINK),
                                move |el, t| el.when(((t * 4.) as usize) % 2 == 0, |el| el.bg(flash)),
                            ),
                        )
                    })
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        this.click(ix, event.click_count() > 1, cx)
                    }))
                    .on_drag(Dragged { path: path.clone(), dir }, |dragged, _, _, cx| FileTree::ghost(dragged, cx))
                    .drag_over::<Dragged>(move |style, _, _, _| style.bg(drop_bg))
                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_bg))
                    .when_some(closed_folder, |el, folder| {
                        el.on_drag_move(cx.listener(move |this, e: &DragMoveEvent<Dragged>, window, cx| {
                            if e.bounds.contains(&e.event.position) {
                                this.drag_over_folder(folder.clone(), e.bounds, window, cx);
                            }
                        }))
                    })
                    .when_some(spring, |el, folder| {
                        el.on_drag_move(cx.listener(move |this, e: &DragMoveEvent<ExternalPaths>, window, cx| {
                            if e.bounds.contains(&e.event.position) {
                                this.drag_over_folder(folder.clone(), e.bounds, window, cx);
                            }
                        }))
                    })
                    .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                        cx.stop_propagation();
                        this.drop_entry(dragged, into.clone(), window, cx)
                    }))
                    .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                        cx.stop_propagation();
                        this.drop_external(paths, into_external.clone(), window, cx)
                    }))
                    .context_menu(move |menu, _, _| edit::menu(menu, tree.clone(), path.clone(), dir, root.clone()))
                    .into_any_element()
            })
            .collect()
    }
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.root.file_name().unwrap_or(self.root.as_os_str()).to_string_lossy().to_uppercase();
        let actions = self.render_actions(cx);
        v_flex()
            .size_full()
            .group("explorer")
            .bg(cx.theme().sidebar)
            .key_context("FileTree")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CancelEdit, window, cx| this.cancel(window, cx)))
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.redo(window, cx)))
            // Clicking in the tree gives it the keyboard, so ⌘Z undoes file changes here.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.focus.focus(window, cx)))
            // A drop below the last row lands in the root.
            .on_drop(cx.listener(|this, dragged: &Dragged, window, cx| {
                let root = this.root.clone();
                this.drop_entry(dragged, root, window, cx)
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                let root = this.root.clone();
                this.drop_external(paths, root, window, cx)
            }))
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
                    .pl_3()
                    .pr_1()
                    .gap_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(actions),
            )
            .child(
                uniform_list(
                    "tree",
                    self.list_len(),
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
            // Rows have their own menu; the header and the empty space below act on the root.
            .context_menu({
                let (tree, root) = (cx.entity().downgrade(), self.root.clone());
                move |menu, _, _| edit::menu(menu, tree.clone(), root.clone(), true, root.clone())
            })
    }
}
