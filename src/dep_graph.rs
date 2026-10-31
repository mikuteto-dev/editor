//! The dependency graph drawn as boxes and curves: the package on the left, then each depth of
//! dependencies in its own column. Selecting a package lights the paths that pull it in.

use std::{cell::Cell, collections::VecDeque, rc::Rc, sync::Arc};

use gpui_kit::{
    component::{
        ActiveTheme, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    prelude::FluentBuilder,
    *,
};

use crate::licenses::{Family, Report, Verdict, family, verdict};

const NODE_W: f32 = 200.;
const NODE_H: f32 = 44.;
const COL_W: f32 = 290.;
const ROW_H: f32 = 58.;

#[derive(Clone, Copy)]
struct Camera {
    offset: Point<f32>,
    zoom: f32,
    fitted: bool,
}

pub struct DepGraph {
    report: Arc<Report>,
    /// Node 0 is the package itself; node i + 1 is `report.deps[i]`.
    names: Vec<SharedString>,
    children: Rc<Vec<Vec<usize>>>,
    parents: Vec<Vec<usize>>,
    pos: Rc<Vec<Point<f32>>>,
    verdicts: Vec<Verdict>,
    /// Whether a license problem sits in or below each node.
    troubled: Rc<Vec<bool>>,
    selected: Option<usize>,
    /// The selected node, everything on its paths from the package, and what it uses directly.
    lit: Rc<Vec<bool>>,
    camera: Rc<Cell<Camera>>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    last_mouse: Option<Point<Pixels>>,
    dragged: bool,
    search: Entity<InputState>,
    _search: Subscription,
}

impl DepGraph {
    pub fn new(report: Arc<Report>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let n = report.deps.len() + 1;
        let mut children = vec![Vec::new(); n];
        children[0] = if report.roots.is_empty() { (1..n).collect() } else { report.roots.iter().map(|r| r + 1).collect() };
        for &(from, to) in &report.edges {
            children[from + 1].push(to + 1);
        }
        for list in &mut children {
            list.sort_unstable();
            list.dedup();
        }
        let mut parents = vec![Vec::new(); n];
        for (from, list) in children.iter().enumerate() {
            for &to in list {
                parents[to].push(from);
            }
        }
        let own = report.own.as_deref();
        let verdicts: Vec<Verdict> = std::iter::once(Verdict::Ok)
            .chain(report.deps.iter().map(|d| if d.shipped { verdict(own, d.license.as_deref()) } else { Verdict::Ok }))
            .collect();
        let names: Vec<SharedString> = std::iter::once("This package".into())
            .chain(report.deps.iter().map(|d| d.name.clone().into()))
            .collect();
        let pos = layout(&children, &names);
        let troubled = troubled(&children, &verdicts);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find package"));
        let _search = cx.subscribe(&search, |this: &mut Self, input, e: &InputEvent, cx| {
            let next = matches!(e, InputEvent::PressEnter { .. });
            if matches!(e, InputEvent::Change) || next {
                let query = input.read(cx).value().to_lowercase();
                this.find(&query, next, cx);
            }
        });
        Self {
            report,
            names,
            lit: Rc::new(vec![false; n]),
            children: Rc::new(children),
            parents,
            pos: Rc::new(pos),
            verdicts,
            troubled: Rc::new(troubled),
            selected: None,
            camera: Rc::new(Cell::new(Camera { offset: point(0., 0.), zoom: 1., fitted: false })),
            bounds: Rc::default(),
            last_mouse: None,
            dragged: false,
            search,
            _search,
        }
    }

    fn problems(&self) -> usize {
        self.verdicts.iter().filter(|v| **v != Verdict::Ok).count()
    }

    fn find(&mut self, query: &str, next: bool, cx: &mut Context<Self>) {
        if query.is_empty() {
            return;
        }
        let start = if next { self.selected.map_or(0, |s| s + 1) } else { 0 };
        let n = self.names.len();
        let hit = (0..n).map(|i| (start + i) % n).find(|&i| self.names[i].to_lowercase().contains(query));
        if let Some(hit) = hit {
            self.select(Some(hit), true, cx);
        }
    }

    fn select(&mut self, node: Option<usize>, center: bool, cx: &mut Context<Self>) {
        self.selected = node;
        self.lit = Rc::new(node.map(|n| lit(n, &self.children, &self.parents)).unwrap_or_else(|| vec![false; self.names.len()]));
        if let (Some(node), true) = (node, center) {
            let mut cam = self.camera.get();
            cam.zoom = cam.zoom.max(0.9);
            let size = self.bounds.get().size;
            let p = self.pos[node];
            cam.offset = point(
                f32::from(size.width) / 2. - (p.x + NODE_W / 2.) * cam.zoom,
                f32::from(size.height) / 2. - (p.y + NODE_H / 2.) * cam.zoom,
            );
            self.camera.set(cam);
        }
        cx.notify();
    }

    fn zoom_at(&mut self, at: Point<Pixels>, factor: f32, cx: &mut Context<Self>) {
        let mut cam = self.camera.get();
        let origin = self.bounds.get().origin;
        let p = point(f32::from(at.x - origin.x), f32::from(at.y - origin.y));
        let zoom = (cam.zoom * factor).clamp(0.1, 2.5);
        // Keep the point under the cursor where it is.
        cam.offset = point(p.x - (p.x - cam.offset.x) * zoom / cam.zoom, p.y - (p.y - cam.offset.y) * zoom / cam.zoom);
        cam.zoom = zoom;
        self.camera.set(cam);
        cx.notify();
    }

    fn pan(&mut self, by: Point<Pixels>, cx: &mut Context<Self>) {
        let mut cam = self.camera.get();
        cam.offset = point(cam.offset.x + f32::from(by.x), cam.offset.y + f32::from(by.y));
        self.camera.set(cam);
        cx.notify();
    }

    fn fit(&mut self, cx: &mut Context<Self>) {
        let mut cam = self.camera.get();
        cam.fitted = false;
        self.camera.set(cam);
        cx.notify();
    }

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
                Some(
                    card.on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if !this.dragged {
                                this.select(Some(i), false, cx);
                            }
                        })),
                )
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

/// Columns by depth from the package, rows ordered to keep curves from crossing.
fn layout(children: &[Vec<usize>], names: &[SharedString]) -> Vec<Point<f32>> {
    let n = children.len();
    let mut depth = vec![usize::MAX; n];
    depth[0] = 0;
    let mut queue = VecDeque::from([0]);
    while let Some(node) = queue.pop_front() {
        for &child in &children[node] {
            if depth[child] == usize::MAX {
                depth[child] = depth[node] + 1;
                queue.push_back(child);
            }
        }
    }
    for d in &mut depth {
        if *d == usize::MAX {
            *d = 1;
        }
    }
    let columns = depth.iter().max().copied().unwrap_or(0) + 1;
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); columns];
    for (node, &d) in depth.iter().enumerate() {
        layers[d].push(node);
    }
    for layer in &mut layers {
        layer.sort_by(|a, b| names[*a].cmp(&names[*b]));
    }
    let mut row = vec![0f32; n];
    let place = |layers: &[Vec<usize>], row: &mut [f32]| {
        for layer in layers {
            for (i, &node) in layer.iter().enumerate() {
                row[node] = i as f32 - layer.len() as f32 / 2.;
            }
        }
    };
    place(&layers, &mut row);
    let mut parents = vec![Vec::new(); n];
    for (from, list) in children.iter().enumerate() {
        for &to in list {
            if depth[from] < depth[to] {
                parents[to].push(from);
            }
        }
    }
    // A few barycenter sweeps: each node moves toward the average row of what pulls it in.
    for _ in 0..4 {
        for layer in layers.iter_mut().skip(1) {
            let key = |node: &usize| {
                let p = &parents[*node];
                if p.is_empty() { row[*node] } else { p.iter().map(|q| row[*q]).sum::<f32>() / p.len() as f32 }
            };
            layer.sort_by(|a, b| key(a).total_cmp(&key(b)));
        }
        place(&layers, &mut row);
    }
    (0..n).map(|node| point(depth[node] as f32 * COL_W, row[node] * ROW_H)).collect()
}

fn fit(pos: &[Point<f32>], size: Size<Pixels>) -> Camera {
    let (w, h) = (f32::from(size.width), f32::from(size.height));
    let min_x = pos.iter().map(|p| p.x).fold(f32::MAX, f32::min);
    let max_x = pos.iter().map(|p| p.x).fold(f32::MIN, f32::max) + NODE_W;
    let min_y = pos.iter().map(|p| p.y).fold(f32::MAX, f32::min);
    let max_y = pos.iter().map(|p| p.y).fold(f32::MIN, f32::max) + NODE_H;
    let pad = 40.;
    // Small graphs fill the view; large ones stay readable and start from the package on the left.
    let zoom = ((w - pad * 2.) / (max_x - min_x)).min((h - pad * 2.) / (max_y - min_y)).clamp(0.6, 1.);
    let x = if (max_x - min_x) * zoom <= w { (w - (max_x + min_x) * zoom) / 2. } else { pad - min_x * zoom };
    let y = h / 2. - pos[0].y.max(min_y).min(max_y) * zoom - NODE_H * zoom / 2.;
    let y = if (max_y - min_y) * zoom <= h { (h - (max_y + min_y) * zoom) / 2. } else { y };
    Camera { offset: point(x, y), zoom, fitted: true }
}

/// For each node, whether it or anything below it has a problem; cycles count once.
fn troubled(children: &[Vec<usize>], verdicts: &[Verdict]) -> Vec<bool> {
    let n = children.len();
    let mut state = vec![0u8; n]; // 0 unvisited, 1 on the stack, 2 done
    let mut result = vec![false; n];
    fn visit(ix: usize, children: &[Vec<usize>], verdicts: &[Verdict], state: &mut [u8], result: &mut [bool]) -> bool {
        match state[ix] {
            2 => return result[ix],
            1 => return false,
            _ => {}
        }
        state[ix] = 1;
        let mut found = verdicts[ix] != Verdict::Ok;
        for &child in &children[ix] {
            found |= visit(child, children, verdicts, state, result);
        }
        state[ix] = 2;
        result[ix] = found;
        found
    }
    for ix in 0..n {
        visit(ix, children, verdicts, &mut state, &mut result);
    }
    result
}

/// The node, every node on a path from the package down to it, and what it uses directly.
fn lit(node: usize, children: &[Vec<usize>], parents: &[Vec<usize>]) -> Vec<bool> {
    let mut lit = vec![false; children.len()];
    let mut queue = VecDeque::from([node]);
    while let Some(n) = queue.pop_front() {
        if !std::mem::replace(&mut lit[n], true) {
            queue.extend(&parents[n]);
        }
    }
    for &child in &children[node] {
        lit[child] = true;
    }
    lit
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

#[std::prelude::rust_2024::test]
fn lays_out_by_depth_and_lights_paths() {
    // package → a → c, package → b → c, and c → d.
    let children = vec![vec![1, 2], vec![3], vec![3], vec![4], vec![]];
    let names: Vec<SharedString> = ["pkg", "a", "b", "c", "d"].map(Into::into).to_vec();
    let pos = layout(&children, &names);
    let columns: Vec<f32> = pos.iter().map(|p| p.x / COL_W).collect();
    assert_eq!(columns, [0., 1., 1., 2., 3.]);
    assert!(pos[1].y < pos[2].y, "a above b, by name");

    let mut parents = vec![Vec::new(); 5];
    for (from, list) in children.iter().enumerate() {
        for &to in list {
            parents[to].push(from);
        }
    }
    // Selecting c lights both paths that pull it in and what it uses, not its siblings' other children.
    assert_eq!(lit(3, &children, &parents), [true, true, true, true, true]);
    assert_eq!(lit(1, &children, &parents), [true, true, false, true, false]);

    let verdicts = vec![Verdict::Ok, Verdict::Ok, Verdict::Ok, Verdict::Ok, Verdict::Conflict("x".into())];
    assert_eq!(troubled(&children, &verdicts), [true, true, true, true, true]);
}
