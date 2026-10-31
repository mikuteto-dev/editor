use std::collections::VecDeque;

use gpui_kit::*;

use crate::licenses::Verdict;

pub(super) const NODE_W: f32 = 200.;
pub(super) const NODE_H: f32 = 44.;
const COL_W: f32 = 290.;
const ROW_H: f32 = 58.;

#[derive(Clone, Copy)]
pub(super) struct Camera {
    pub offset: Point<f32>,
    pub zoom: f32,
    pub fitted: bool,
}

/// Columns by depth from the package, rows ordered to keep curves from crossing.
pub(super) fn layout(children: &[Vec<usize>], names: &[SharedString]) -> Vec<Point<f32>> {
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

pub(super) fn fit(pos: &[Point<f32>], size: Size<Pixels>) -> Camera {
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
pub(super) fn troubled(children: &[Vec<usize>], verdicts: &[Verdict]) -> Vec<bool> {
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

pub(super) fn lit(node: usize, children: &[Vec<usize>], parents: &[Vec<usize>]) -> Vec<bool> {
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
