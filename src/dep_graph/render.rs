use gpui_kit::{
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::Input,
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use super::{
    DepGraph,
    layout::{NODE_H, NODE_W, fit},
};
use crate::licenses::{Family, Verdict, family};

impl DepGraph {
    fn render_nodes(&self, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let theme = cx.theme();
        let cam = self.camera.get();
        let size = self.bounds.get().size;
        let (w, h) = (NODE_W * cam.zoom, NODE_H * cam.zoom);
        let any_lit = self.selected.is_some();
        (0..self.names.len())
            .filter_map(|i| {
                let p = self.pos[i];
                let (x, y) = (p.x * cam.zoom + cam.offset.x, p.y * cam.zoom + cam.offset.y);
                // Only what is on screen becomes an element.
                let visible = size.width <= px(0.) || x + w >= 0. && y + h >= 0. && x <= f32::from(size.width) && y <= f32::from(size.height);
                if !visible {
                    return None;
                }
                let z = cam.zoom;
                let dep = (i > 0).then(|| &self.report.deps[i - 1]);
                let dev = dep.is_some_and(|d| !d.shipped);
                let selected = self.selected == Some(i);
                let license = match dep {
                    Some(d) => d.license.clone(),
                    None => self.report.own.clone(),
                };
                // Color tells how much the license asks; the border tells whether it fits.
                let tint = match family(license.as_deref()) {
                    Family::Permissive => theme.green,
                    Family::WeakCopyleft => theme.blue,
                    Family::Copyleft => theme.magenta,
                    Family::Unknown => theme.yellow,
                };
                let alarm = match &self.verdicts[i] {
                    Verdict::Ok => None,
                    Verdict::Caution(_) | Verdict::Unknown(_) => Some(theme.warning),
                    Verdict::Conflict(_) => Some(theme.danger),
                };
                let border = if selected { theme.primary } else { alarm.unwrap_or(theme.border) };
                let faded = any_lit && !self.lit[i];
                let card = div()
                    .id(("node", i))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(8. * z))
                    .overflow_hidden()
                    .border(px(if selected || alarm.is_some() { 2. } else { 1. }))
                    .border_color(border)
                    .bg(if i == 0 { theme.accent } else { theme.popover })
                    .when(selected, |el| el.shadow_lg())
                    .when(!selected, |el| el.shadow_sm())
                    .hover(move |el| el.border_color(if selected { border } else { tint }))
                    .when(dev, |el| el.opacity(0.55))
                    .when(faded, |el| el.opacity(0.15));
                // Zoomed far out, a card is just its color.
                let card = if z < 0.45 {
                    card.bg(tint.opacity(0.55))
                } else {
                    let version = dep.map(|d| d.version.clone()).unwrap_or_default();
                    card.flex()
                        .child(div().w(px(4. * z)).h_full().flex_none().bg(tint))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .justify_center()
                                .gap(px(3. * z))
                                .px(px(9. * z))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.5 * z))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(theme.foreground)
                                        .child(self.names[i].clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap(px(6. * z))
                                        .min_w_0()
                                        .text_size(px(10.5 * z))
                                        .child(
                                            div()
                                                .flex_none()
                                                .px(px(5. * z))
                                                .rounded_full()
                                                .bg(tint.opacity(0.16))
                                                .text_color(tint)
                                                .child(license.unwrap_or_else(|| "no license".into())),
                                        )
                                        .child(div().truncate().text_color(theme.muted_foreground).child(version))
                                        .when(dev, |el| el.child(div().flex_none().text_color(theme.muted_foreground).child("dev"))),
                                ),
                        )
                        .children(alarm.map(|color| {
                            div()
                                .flex_none()
                                .w(px(18. * z))
                                .h_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(color.opacity(0.18))
                                .text_size(px(12. * z))
                                .font_weight(FontWeight::BOLD)
                                .text_color(color)
                                .child("!")
                        }))
                };
                Some(card.on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if !this.dragged {
                        this.select(Some(i), false, cx);
                    }
                })))
            })
            .collect()
    }

    fn render_edges(&self, cx: &App) -> Canvas<()> {
        let theme = cx.theme();
        let colors = [theme.border.opacity(0.7), theme.danger.opacity(0.55), theme.primary];
        let (children, pos, troubled, lit) = (self.children.clone(), self.pos.clone(), self.troubled.clone(), self.lit.clone());
        let any_lit = self.selected.is_some();
        let (camera, bounds) = (self.camera.clone(), self.bounds.clone());
        let (fit_camera, fit_pos) = (self.camera.clone(), self.pos.clone());
        canvas(
            move |b, window, _| {
                bounds.set(b);
                // The view's size is first known here, so the first fit happens here too.
                if !fit_camera.get().fitted && b.size.width > px(0.) {
                    fit_camera.set(fit(&fit_pos, b.size));
                    window.refresh();
                }
            },
            move |b, _, window, _| {
                let cam = camera.get();
                let at = |p: Point<f32>| point(b.origin.x + px(p.x * cam.zoom + cam.offset.x), b.origin.y + px(p.y * cam.zoom + cam.offset.y));
                let (width, height) = (f32::from(b.size.width), f32::from(b.size.height));
                // One path per color: thousands of curves tessellate once instead of one by one.
                let mut builders = [PathBuilder::stroke(px(1.)), PathBuilder::stroke(px(1.5)), PathBuilder::stroke(px(2.))];
                let mut used = [false; 3];
                for (from, list) in children.iter().enumerate() {
                    for &to in list {
                        let a = point(pos[from].x + NODE_W, pos[from].y + NODE_H / 2.);
                        let z = point(pos[to].x, pos[to].y + NODE_H / 2.);
                        let (sa, sz) = (at(a), at(z));
                        let (ax, ay, zx, zy) = (f32::from(sa.x - b.origin.x), f32::from(sa.y - b.origin.y), f32::from(sz.x - b.origin.x), f32::from(sz.y - b.origin.y));
                        if ax.max(zx) < 0. || ax.min(zx) > width || ay.max(zy) < 0. || ay.min(zy) > height {
                            continue;
                        }
                        let kind = if any_lit && lit[from] && lit[to] {
                            2
                        } else if troubled[to] && troubled[from] {
                            1
                        } else if any_lit {
                            continue;
                        } else {
                            0
                        };
                        let bend = px(((z.x - a.x).abs().max(60.) / 2.) * cam.zoom);
                        builders[kind].move_to(sa);
                        builders[kind].cubic_bezier_to(sz, point(sa.x + bend, sa.y), point(sz.x - bend, sz.y));
                        used[kind] = true;
                    }
                }
                for (kind, builder) in builders.into_iter().enumerate() {
                    if used[kind]
                        && let Ok(path) = builder.build()
                    {
                        window.paint_path(path, colors[kind]);
                    }
                }
            },
        )
        .absolute()
        .size_full()
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme().clone();
        let node = self.selected.unwrap_or(0);
        let (version, license, shipped) = match node {
            0 => (String::new(), self.report.own.clone(), true),
            i => {
                let d = &self.report.deps[i - 1];
                (d.version.clone(), d.license.clone(), d.shipped)
            }
        };
        let note = match &self.verdicts[node] {
            Verdict::Ok => None,
            Verdict::Caution(why) | Verdict::Unknown(why) => Some((theme.warning, why.clone())),
            Verdict::Conflict(why) => Some((theme.danger, why.clone())),
        };
        let list = |title: &str, nodes: &[usize], cx: &mut Context<Self>| {
            let rows: Vec<_> = nodes
                .iter()
                .take(200)
                .map(|&n| {
                    let color = match &self.verdicts[n] {
                        Verdict::Ok => theme.foreground,
                        Verdict::Caution(_) | Verdict::Unknown(_) => theme.warning,
                        Verdict::Conflict(_) => theme.danger,
                    };
                    div()
                        .id(("link", n))
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .truncate()
                        .text_color(color)
                        .hover(|el| el.bg(theme.list_hover))
                        .child(self.names[n].clone())
                        .on_click(cx.listener(move |this, _, _, cx| this.select(Some(n), true, cx)))
                })
                .collect();
            v_flex()
                .gap_0p5()
                .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{} · {}", title.to_uppercase(), nodes.len())))
                .children(rows)
        };
        let used_by = list("Used by", &self.parents[node], cx);
        let uses = list("Uses", &self.children[node], cx);
        v_flex()
            .id("detail")
            .w(px(300.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .p_3()
            .gap_3()
            .border_l_1()
            .border_color(theme.border)
            .text_sm()
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(self.names[node].clone()))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(version)
                            .child(license.unwrap_or_else(|| "no license".into()))
                            .when(!shipped, |el| el.child("dev only")),
                    )
                    .children(note.map(|(color, why)| div().text_xs().text_color(color).child(why))),
            )
            .when(node > 0, |el| el.child(used_by))
            .child(uses)
    }
}

impl Render for DepGraph {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let summary = format!(
            "{} · {} packages · {} to check",
            self.report.own.as_deref().unwrap_or("no license"),
            self.report.deps.len(),
            self.problems(),
        );
        let (border, bg, muted) = (theme.border, theme.background, theme.muted_foreground);
        let nodes = self.render_nodes(cx);
        let edges = self.render_edges(cx);
        let detail = self.render_detail(cx);
        v_flex()
            .size_full()
            .text_sm()
            .child(
                h_flex()
                    .h(px(40.))
                    .flex_none()
                    .px_3()
                    .gap_3()
                    .border_b_1()
                    .border_color(border)
                    .child(div().w(px(220.)).child(Input::new(&self.search).small()))
                    .child(div().flex_1().text_color(muted).child(summary))
                    .child(Button::new("fit").label("Fit").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| this.fit(cx)))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("canvas")
                            .relative()
                            .flex_1()
                            .h_full()
                            .overflow_hidden()
                            .bg(bg)
                            .child(edges)
                            .children(nodes)
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, e: &MouseDownEvent, _, _| {
                                    this.last_mouse = Some(e.position);
                                    this.dragged = false;
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                                let Some(last) = this.last_mouse.filter(|_| e.pressed_button == Some(MouseButton::Left)) else {
                                    return;
                                };
                                let by = e.position - last;
                                this.dragged |= f32::from(by.x).abs() + f32::from(by.y).abs() > 2.;
                                this.last_mouse = Some(e.position);
                                this.pan(by, cx);
                            }))
                            .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, _| this.last_mouse = None))
                            .on_scroll(cx.listener(|this, e: &ScrollEvent, _, cx| {
                                let delta = e.delta.pixel_delta(px(20.));
                                if e.modifiers.platform || e.modifiers.control {
                                    this.zoom_at(e.position, (1. + f32::from(delta.y) / 300.).clamp(0.5, 2.), cx);
                                } else {
                                    this.pan(delta, cx);
                                }
                            }))
                            .on_pinch(cx.listener(|this, e: &PinchEvent, _, cx| this.zoom_at(e.position, 1. + e.delta, cx)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.dragged {
                                    this.select(None, false, cx);
                                }
                            })),
                    )
                    .child(detail),
            )
    }
}
