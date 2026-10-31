mod layout;
mod render;

use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui_kit::{
    component::input::{InputEvent, InputState},
    *,
};

use crate::licenses::{Report, Verdict, verdict};
use layout::{Camera, NODE_H, NODE_W, layout, lit, troubled};

pub struct DepGraph {
    report: Arc<Report>,
    /// Node 0 is the package itself; node i + 1 is `report.deps[i]`.
    names: Vec<SharedString>,
    children: Rc<Vec<Vec<usize>>>,
    parents: Vec<Vec<usize>>,
    pos: Rc<Vec<Point<f32>>>,
    verdicts: Vec<Verdict>,
    troubled: Rc<Vec<bool>>,
    selected: Option<usize>,
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
}
