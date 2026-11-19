use crate::metrics::Ctx;
use crate::scene::{Model, VW, Weights, normalize_vw, prune_vw};
use crate::transfer::{TriIndex, inpaint_from_matches};
use rayon::prelude::*;

// ------------------------------------------------------------------ helpers

/// Weight Laplacian smoothing restricted to `mask`ed vertices.
pub fn smooth(model: &Model, w: &Weights, mask: &[bool], iters: usize, lambda: f64) -> Weights {
    let mut cur = w.clone();
    for _ in 0..iters {
        cur = (0..model.nverts())
            .into_par_iter()
            .map(|v| {
                if !mask[v] || model.adj[v].is_empty() {
                    return cur[v].clone();
                }
                let k = model.adj[v].len() as f64;
                let mut acc: VW = cur[v].iter().map(|&(j, x)| (j, x * (1.0 - lambda))).collect();
                for &u in &model.adj[v] {
                    acc.extend(cur[u as usize].iter().map(|&(j, x)| (j, x * lambda / k)));
                }
                normalize_vw(acc)
            })
            .collect();
    }
    cur
}

/// Repair band widths, in feature sizes (`Ctx::feature`, the median limb
/// radius), never in edge rings: a ring-count band is half as wide on a
/// mesh twice as dense, and tears there. Calibrated on the test mannequin
/// the ring counts were tuned on (P2; mean edge 0.47 feature sizes, cape
/// edge 0.8), so there each band is as wide as before (old ring counts in
/// brackets).
pub mod widths {
    /// Margin around flagged vertices that a fix may change (6 rings).
    pub const MARGIN: f64 = 2.8;
    /// Seam blend between candidates (and the input): the Gaussian width
    /// of `blend` (what 3 smoothing passes spread on the test mannequin).
    pub const SEAM: f64 = 0.4;
    /// Clean-up: margin around tears and collapses (3 rings) and its
    /// smoothing width (12 passes).
    pub const TORN_MARGIN: f64 = 1.4;
    pub const TORN_SMOOTH: f64 = 0.8;
    /// Geodesic binding's smoothing (4 passes).
    pub const GEODESIC_SMOOTH: f64 = 0.5;
    /// Piece (cape, skirt) smoothing after transfer (2 passes).
    pub const PIECE_SMOOTH: f64 = 0.57;
    /// Margin that optimize may change around flagged vertices (4 rings).
    pub const OPTIMIZE_MARGIN: f64 = 1.9;
}

/// Vertices within rest-pose edge-path distance `dist` of `seeds`,
/// travelling only through `within` (everywhere when `None`). The metric
/// counterpart of `dilate`.
pub fn grow(model: &Model, seeds: &[bool], dist: f64, within: Option<&[bool]>) -> Vec<bool> {
    distances_within(model, seeds, dist, within).iter().map(|x| x.is_finite()).collect()
}

/// Rest-pose edge-path distance from `seeds`, infinite beyond `max`.
pub fn distances(model: &Model, seeds: &[bool], max: f64) -> Vec<f64> {
    distances_within(model, seeds, max, None)
}

fn distances_within(model: &Model, seeds: &[bool], max: f64, within: Option<&[bool]>) -> Vec<f64> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = model.nverts();
    let mut d = vec![f64::INFINITY; n];
    let mut heap = BinaryHeap::new();
    for v in 0..n {
        if seeds[v] {
            d[v] = 0.0;
            heap.push(Reverse((0u64, v)));
        }
    }
    // f64 >= 0 orders like its bit pattern.
    while let Some(Reverse((bits, v))) = heap.pop() {
        let dv = f64::from_bits(bits);
        if dv > d[v] {
            continue;
        }
        for &u in &model.adj[v] {
            let u = u as usize;
            if within.is_some_and(|m| !m[u]) {
                continue;
            }
            let du = dv + (model.rest[u] - model.rest[v]).len();
            if du < d[u] && du <= max {
                d[u] = du;
                heap.push(Reverse((du.to_bits(), u)));
            }
        }
    }
    d
}

/// Smoothing passes (`smooth` with lambda 0.5) that spread a weight like a
/// Gaussian of width `sigma`: each pass moves it a quarter of a squared edge
/// on average, so n passes spread `h * sqrt(n) / 2` (h = mean rest edge
/// length among `mask`ed vertices).
pub fn passes(model: &Model, mask: &[bool], sigma: f64) -> usize {
    let (mut sum, mut cnt) = (0.0, 0usize);
    for e in &model.edges {
        if mask[e[0] as usize] && mask[e[1] as usize] {
            sum += (model.rest[e[0] as usize] - model.rest[e[1] as usize]).len();
            cnt += 1;
        }
    }
    if cnt == 0 || sum <= 0.0 {
        return 0;
    }
    let h = sum / cnt as f64;
    ((4.0 * sigma * sigma / (h * h)).round() as usize).min(2000)
}

pub fn dilate(model: &Model, mask: &[bool], rings: usize) -> Vec<bool> {
    let mut m = mask.to_vec();
    for _ in 0..rings {
        let prev = m.clone();
        for v in 0..model.nverts() {
            if !prev[v] && model.adj[v].iter().any(|&u| prev[u as usize]) {
                m[v] = true;
            }
        }
    }
    m
}

/// Keeps only connected flagged clusters of at least `min` vertices.
pub(super) fn drop_small_clusters(model: &Model, mask: &[bool], min: usize) -> Vec<bool> {
    let n = model.nverts();
    let mut out = vec![false; n];
    let mut seen = vec![false; n];
    for s in 0..n {
        if !mask[s] || seen[s] {
            continue;
        }
        let mut comp = vec![s];
        seen[s] = true;
        let mut i = 0;
        while i < comp.len() {
            for &u in &model.adj[comp[i]] {
                let u = u as usize;
                if mask[u] && !seen[u] {
                    seen[u] = true;
                    comp.push(u);
                }
            }
            i += 1;
        }
        // A failing piece is flagged whole regardless of size.
        if comp.len() >= min || is_piece(model, s) {
            for v in comp {
                out[v] = true;
            }
        }
    }
    out
}

pub(super) fn prune_all(w: &Weights) -> Weights {
    w.par_iter().map(|vw| prune_vw(vw, 4, 0.01)).collect()
}

pub(super) fn is_piece(model: &Model, v: usize) -> bool {
    model.parts[model.vpart[v] as usize].piece
}

/// Pieces hang from where they attach: each piece follows the bones at
/// its attached vertices (close to the body, facing the same way) and
/// their parent chain, never sibling limbs. Piece vertices take
/// nearest-surface weights from body surface dominated by that chain
/// (a cape follows the spine down to the hips, not the arms or thighs).
/// A piece with no attached vertices falls back to robust transfer.
pub(super) fn pieces_from_body(model: &Model, ctx: &Ctx, w: &Weights) -> Weights {
    let mut out = w.clone();
    if ctx.piece_transfer.is_empty() {
        return out;
    }
    let robust = inpaint_from_matches(&model.adj, &ctx.piece_transfer, w);
    let sk = &model.skel;
    for (pi, part) in model.parts.iter().enumerate() {
        if !part.piece {
            continue;
        }
        let verts: Vec<usize> = (0..model.nverts()).filter(|&v| model.vpart[v] as usize == pi).collect();
        let mut allowed = vec![false; model.njoints()];
        for m in ctx.piece_matches.iter().filter(|m| m.3 && model.vpart[m.0 as usize] as usize == pi) {
            for &(j, x) in &crate::transfer::blend(w, m.1, m.2) {
                if x > 0.1 {
                    let mut k = Some(j as usize);
                    while let Some(b) = k {
                        if allowed[b] {
                            break;
                        }
                        allowed[b] = true;
                        k = sk.jparent[b];
                    }
                }
            }
        }
        let chain_tris: Vec<[u32; 3]> = model
            .tris
            .iter()
            .filter(|t| t.iter().all(|&i| !is_piece(model, i as usize) && w[i as usize].first().is_some_and(|e| allowed[e.0 as usize])))
            .copied()
            .collect();
        if chain_tris.is_empty() {
            for &v in &verts {
                out[v] = robust[v].clone();
            }
            continue;
        }
        let index = TriIndex::new(&model.rest, chain_tris);
        for &v in &verts {
            out[v] = match index.closest(model.rest[v]) {
                Some(h) => crate::transfer::blend(w, index.tri(h.tri), h.bary),
                None => robust[v].clone(),
            };
        }
    }
    let piece_mask: Vec<bool> = (0..model.nverts()).map(|v| is_piece(model, v)).collect();
    let n = passes(model, &piece_mask, widths::PIECE_SMOOTH * ctx.feature);
    smooth(model, &out, &piece_mask, n, 0.5)
}

/// Bones that carry weight anywhere in the input (the rig's deform set).
pub(super) fn deform_bones(model: &Model) -> Vec<bool> {
    let mut used = vec![false; model.njoints()];
    for vw in &model.weights {
        for &(j, x) in vw {
            if x > 0.01 {
                used[j as usize] = true;
            }
        }
    }
    if !used.iter().any(|&u| u) {
        used.iter_mut().for_each(|u| *u = true);
    }
    used
}
