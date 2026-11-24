//! Geodesic distance from each bone to each vertex, measured through a
//! cut-cell geodesic graph of the mesh (extending Dionne & de Lasa 2013 with
//! exact surface-cut grid edges so narrow sub-voxel air gaps never bridge).
//!
//! Six half-rays along the grid axes classify strictly interior grid nodes
//! (`votes >= 4`, surviving small mesh openings) and record exact ray-triangle
//! intervals per shell along every axis edge. Axis edges stay open across
//! overlapping closed shells (where union parity stays `> 0`) and are cut
//! whenever the segment exits into air (even a sub-voxel gap on a single
//! watertight shell). Diagonal edges stay open where locally connected by
//! axis edges and free of triangle intersections, preserving 26-connected
//! angular accuracy without cutting corners across air gaps. Surface vertices
//! attach only to unobstructed interior nodes and stay linked along mesh
//! edges so thin parts narrower than a voxel remain reachable.

use crate::math::{Vec3, segment_distance, v3};
use crate::scene::Model;
use rayon::prelude::*;
use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Small deterministic grid-coordinate jitter shared by all three axis ray
/// families so every `(x, y, z)` node sits at one exact 3D point off any
/// grid-aligned triangle edge or vertex.
const RAY_EPS: [f64; 3] = [0.001_371_9, 0.002_917_3, 0.004_193_7];

pub struct Grid {
    pub n: [usize; 3],
    pub origin: Vec3,
    pub h: f64,
    /// 0 = empty, 1 = surface, 2 = interior.
    pub cell: Vec<u8>,
}

impl Grid {
    pub fn idx(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.n[1] + y) * self.n[0] + x
    }
    pub fn coord(&self, p: Vec3) -> [f64; 3] {
        [(p.x - self.origin.x) / self.h - 0.5, (p.y - self.origin.y) / self.h - 0.5, (p.z - self.origin.z) / self.h - 0.5]
    }
    pub fn cell_of(&self, p: Vec3) -> Option<[usize; 3]> {
        let c = self.coord(p);
        let mut o = [0usize; 3];
        for k in 0..3 {
            let v = c[k].round();
            if v < 0.0 || v as usize >= self.n[k] {
                return None;
            }
            o[k] = v as usize;
        }
        Some(o)
    }
    pub fn center(&self, x: usize, y: usize, z: usize) -> Vec3 {
        self.origin + v3(x as f64 + 0.5, y as f64 + 0.5, z as f64 + 0.5) * self.h
    }
    pub fn solid(&self, i: usize) -> bool {
        self.cell[i] != 0
    }
}

#[inline]
fn node_pos(g: &Grid, x: usize, y: usize, z: usize) -> Vec3 {
    g.origin + v3(x as f64 + 0.5 + RAY_EPS[0], y as f64 + 0.5 + RAY_EPS[1], z as f64 + 0.5 + RAY_EPS[2]) * g.h
}

/// How sealed the body is, from the voxel fill (`voxelize`).
#[derive(Clone, Debug, Default, Serialize)]
pub struct Solid {
    /// Holes in the body's surface (loops of edges with one face).
    pub openings: usize,
    /// The largest opening's extent, as a fraction of the body's size.
    pub largest_opening: f64,
    /// Share of the inside that the outside flood reached through the
    /// openings and only the ray vote put back.
    pub leaked: f64,
    /// Share of the body's inside (or near it) where the rays split three
    /// to three: the fill cannot tell inside from outside there.
    pub contested: f64,
    /// False when too much is contested (`MAX_CONTESTED`) for distances
    /// through the body to be trusted.
    pub trusted: bool,
}

/// More of the inside than this contested and the fill is not trusted.
pub const MAX_CONTESTED: f64 = 0.05;

/// Connected component ID per triangle over shared vertices.
fn tri_shells(nverts: usize, tris: &[[u32; 3]]) -> Vec<u32> {
    let mut parent: Vec<u32> = (0..nverts as u32).collect();
    fn find(p: &mut [u32], mut x: u32) -> u32 {
        while p[x as usize] != x {
            p[x as usize] = p[p[x as usize] as usize];
            x = p[x as usize];
        }
        x
    }
    for t in tris {
        if t.iter().any(|&v| v as usize >= nverts) {
            continue;
        }
        let a = find(&mut parent, t[0]);
        for &v in &t[1..] {
            let b = find(&mut parent, v);
            if a != b {
                parent[b as usize] = a;
            }
        }
    }
    tris.iter().map(|t| if (t[0] as usize) < parent.len() { find(&mut parent, t[0]) } else { 0 }).collect()
}

/// Watertight 2D point-in-triangle test along one projection axis using
/// canonical vertex-index edge ordering so shared edges never leak or double-hit.
#[inline]
#[allow(clippy::too_many_arguments)]
fn ray_tri_2d(t: [u32; 3], a: (f64, f64), b: (f64, f64), c: (f64, f64), d0: f64, d1: f64, d2: f64, p: (f64, f64)) -> Option<f64> {
    let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    if area.abs() < 1e-14 {
        return None;
    }
    let pos_area = area > 0.0;
    let edge = |vi: u32, vj: u32, pa: (f64, f64), pb: (f64, f64)| -> Option<f64> {
        let (s, e, flipped) = if vi < vj { (pa, pb, false) } else { (pb, pa, true) };
        let ec = (e.0 - s.0) * (p.1 - s.1) - (e.1 - s.1) * (p.0 - s.0);
        let want_ge = pos_area != flipped;
        let ok = if want_ge { ec >= 0.0 } else { ec < 0.0 };
        ok.then_some(if flipped { -ec } else { ec })
    };
    let ed0 = edge(t[0], t[1], a, b)?;
    let ed1 = edge(t[1], t[2], b, c)?;
    let ed2 = edge(t[2], t[0], c, a)?;
    Some((ed1 * d0 + ed2 * d1 + ed0 * d2) / area)
}

struct RayData {
    /// Six-ray inside votes per grid node (0..=6).
    votes: Vec<u8>,
    /// Per-cell bitmask: bit `axis` set iff the axis edge from `(x, y, z)` to
    /// `+1` along `axis` never exits the solid union into air.
    axis_open: Vec<u8>,
}

/// Six-ray inside votes per grid node, plus exact surface-cut axis edge flags.
fn ray_analysis(g: &Grid, pos: &[Vec3], tris: &[[u32; 3]], shell: &[u32], use_tri: &dyn Fn(usize) -> bool, compute_edges: bool) -> RayData {
    let total = g.cell.len();
    let mut votes = vec![0u8; total];
    let mut axis_open = if compute_edges { vec![0u8; total] } else { Vec::new() };
    if pos.is_empty() || tris.is_empty() {
        return RayData { votes, axis_open };
    }
    let n = g.n;
    let mut odd = vec![false; pos.len()];
    let mut shell_hits = vec![0u32; pos.len()];

    for axis in 0..3 {
        let (ua, va) = ((axis + 1) % 3, (axis + 2) % 3);
        let (nu, nv, nk) = (n[ua], n[va], n[axis]);
        let (eu, ev, ea) = (RAY_EPS[ua], RAY_EPS[va], RAY_EPS[axis]);
        let mut hits: Vec<Vec<(f64, u32)>> = vec![Vec::new(); nu * nv];

        for (ti, &t) in tris.iter().enumerate() {
            if !use_tri(ti) || t.iter().any(|&v| v as usize >= pos.len()) {
                continue;
            }
            let (p0, p1, p2) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
            if [p0, p1, p2].iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite()) {
                continue;
            }
            let c0 = g.coord(p0);
            let c1 = g.coord(p1);
            let c2 = g.coord(p2);
            let (a, b, cc) = ((c0[ua], c0[va]), (c1[ua], c1[va]), (c2[ua], c2[va]));
            let area = (b.0 - a.0) * (cc.1 - a.1) - (b.1 - a.1) * (cc.0 - a.0);
            if area.abs() < 1e-14 {
                continue;
            }
            let (lo_u, hi_u) = (a.0.min(b.0).min(cc.0), a.0.max(b.0).max(cc.0));
            let (lo_v, hi_v) = (a.1.min(b.1).min(cc.1), a.1.max(b.1).max(cc.1));
            let u0 = (lo_u - eu).ceil().max(0.0) as usize;
            let v0 = (lo_v - ev).ceil().max(0.0) as usize;
            let u1 = (hi_u - eu).floor();
            let v1 = (hi_v - ev).floor();
            if u1 < 0.0 || v1 < 0.0 {
                continue;
            }
            let u1 = (u1 as usize).min(nu - 1);
            let v1 = (v1 as usize).min(nv - 1);
            let sh = shell[ti];
            for iv in v0..=v1 {
                for iu in u0..=u1 {
                    let p = (iu as f64 + eu, iv as f64 + ev);
                    if let Some(depth) = ray_tri_2d(t, a, b, cc, c0[axis], c1[axis], c2[axis], p) {
                        hits[iv * nu + iu].push((depth, sh));
                    }
                }
            }
        }

        let bit = 1u8 << axis;
        for iv in 0..nv {
            for iu in 0..nu {
                let cell = |k: usize| {
                    let mut q = [0usize; 3];
                    q[axis] = k;
                    q[ua] = iu;
                    q[va] = iv;
                    g.idx(q[0], q[1], q[2])
                };
                let list = &mut hits[iv * nu + iu];
                if list.is_empty() {
                    if compute_edges {
                        for k in 0..nk.saturating_sub(1) {
                            axis_open[cell(k)] |= bit;
                        }
                    }
                    continue;
                }
                list.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
                if compute_edges {
                    for &(_, sh) in list.iter() {
                        shell_hits[sh as usize] += 1;
                    }
                }

                // Sweep from low to high: accumulate low-ray votes and cut axis edges.
                let mut count = 0usize;
                let mut even_count = 0usize;
                let mut h = 0usize;
                while h < list.len() && list[h].0 < ea {
                    let sh = list[h].1 as usize;
                    let o = &mut odd[sh];
                    *o = !*o;
                    if *o {
                        count += 1;
                        if compute_edges && shell_hits[sh] & 1 == 0 {
                            even_count += 1;
                        }
                    } else {
                        count -= 1;
                        if compute_edges && shell_hits[sh] & 1 == 0 {
                            even_count -= 1;
                        }
                    }
                    h += 1;
                }
                if count > 0 {
                    votes[cell(0)] += 1;
                }
                for k in 0..nk.saturating_sub(1) {
                    let mut stayed = even_count > 0;
                    let mut had_hit = false;
                    let next_kf = (k + 1) as f64 + ea;
                    while h < list.len() && list[h].0 < next_kf {
                        had_hit = true;
                        let d0 = list[h].0;
                        while h < list.len() && list[h].0 < next_kf && (list[h].0 - d0) <= 1e-7 {
                            let sh = list[h].1 as usize;
                            let o = &mut odd[sh];
                            *o = !*o;
                            if *o {
                                count += 1;
                                if compute_edges && shell_hits[sh] & 1 == 0 {
                                    even_count += 1;
                                }
                            } else {
                                count -= 1;
                                if compute_edges && shell_hits[sh] & 1 == 0 {
                                    even_count -= 1;
                                }
                            }
                            h += 1;
                        }
                        if even_count == 0 {
                            stayed = false;
                        }
                    }
                    if count > 0 {
                        votes[cell(k + 1)] += 1;
                    }
                    if even_count == 0 {
                        stayed = false;
                    }
                    if compute_edges && (!had_hit || stayed) {
                        axis_open[cell(k)] |= bit;
                    }
                }
                for &(_, sh) in list.iter() {
                    odd[sh as usize] = false;
                }

                // Sweep from high to low: accumulate high-ray votes.
                let mut count = 0usize;
                let mut h = list.len();
                for k in (0..nk).rev() {
                    let kf = k as f64 + ea;
                    while h > 0 && list[h - 1].0 > kf {
                        let sh = list[h - 1].1 as usize;
                        let o = &mut odd[sh];
                        *o = !*o;
                        if *o {
                            count += 1;
                        } else {
                            count -= 1;
                        }
                        h -= 1;
                    }
                    if count > 0 {
                        votes[cell(k)] += 1;
                    }
                }
                for &(_, sh) in list.iter() {
                    odd[sh as usize] = false;
                    if compute_edges {
                        shell_hits[sh as usize] = 0;
                    }
                }
            }
        }
    }
    RayData { votes, axis_open }
}

/// Holes in a surface: loops of edges that have one face, as (count,
/// largest loop's bounding-box extent).
fn openings(pos: &[Vec3], tris: &[[u32; 3]], use_tri: &dyn Fn(usize) -> bool) -> (usize, f64) {
    let mut edges: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
    for (ti, t) in tris.iter().enumerate() {
        if !use_tri(ti) || t.iter().any(|&v| v as usize >= pos.len()) {
            continue;
        }
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            *edges.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut parent: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    fn find(p: &mut std::collections::HashMap<u32, u32>, x: u32) -> u32 {
        let mut r = x;
        while let Some(&q) = p.get(&r) {
            if q == r {
                break;
            }
            r = q;
        }
        p.insert(x, r);
        r
    }
    let mut open: Vec<(u32, u32)> = edges.iter().filter(|e| *e.1 == 1).map(|e| *e.0).collect();
    open.sort_unstable();
    for &(a, b) in &open {
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent.insert(rb, ra);
        }
    }
    let verts: Vec<u32> = parent.keys().copied().collect();
    let mut boxes: std::collections::BTreeMap<u32, (Vec3, Vec3)> = std::collections::BTreeMap::new();
    for v in verts {
        let Some(&p) = pos.get(v as usize) else { continue };
        let r = find(&mut parent, v);
        let e = boxes.entry(r).or_insert((p, p));
        e.0 = e.0.min(p);
        e.1 = e.1.max(p);
    }
    let largest = boxes.values().map(|(lo, hi)| (*hi - *lo).max_elem()).fold(0.0, f64::max);
    (boxes.len(), largest)
}

fn voxelize_internal(
    pos: &[Vec3],
    tris: &[[u32; 3]],
    res: usize,
    body: Option<&[bool]>,
    compute_edges: bool,
) -> (Grid, Solid, RayData, Vec<u32>) {
    let res = res.clamp(1, 2048);
    let mut lo = v3(f64::MAX, f64::MAX, f64::MAX);
    let mut hi = -lo;
    let mut any_finite = false;
    for p in pos {
        if p.x.is_finite() && p.y.is_finite() && p.z.is_finite() {
            lo = lo.min(*p);
            hi = hi.max(*p);
            any_finite = true;
        }
    }
    let mut ext = if any_finite { hi - lo } else { Vec3::ZERO };
    if !any_finite || !ext.x.is_finite() || !ext.y.is_finite() || !ext.z.is_finite() {
        lo = Vec3::ZERO;
        ext = Vec3::ZERO;
    }
    let h = (ext.max_elem() / res as f64).max(1e-9);
    let pad = 2.0;
    let origin = lo - v3(pad, pad, pad) * h;
    let axis = |len: f64| -> usize { ((len / h).ceil() as usize + 2 * pad as usize + 1).clamp(1, res + 8) };
    let n = [axis(ext.x), axis(ext.y), axis(ext.z)];
    let mut g = Grid { n, origin, h, cell: vec![0u8; n[0] * n[1] * n[2]] };
    // Surface: sample each triangle densely enough to hit every voxel it crosses.
    for t in tris {
        if t.iter().any(|&i| i as usize >= pos.len()) {
            continue;
        }
        let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
        if [a, b, c].iter().any(|p| !p.x.is_finite() || !p.y.is_finite() || !p.z.is_finite()) {
            continue;
        }
        let lmax = (b - a).len().max((c - a).len()).max((c - b).len());
        if !lmax.is_finite() {
            continue;
        }
        let k = ((lmax / (h * 0.5)).ceil() as usize).clamp(1, res * 4);
        for i in 0..=k {
            for j in 0..=(k - i) {
                let (u, v) = (i as f64 / k as f64, j as f64 / k as f64);
                let p = a + (b - a) * u + (c - a) * v;
                if let Some([x, y, z]) = g.cell_of(p) {
                    let id = g.idx(x, y, z);
                    g.cell[id] = 1;
                }
            }
        }
    }
    for p in pos {
        if let Some([x, y, z]) = g.cell_of(*p) {
            let id = g.idx(x, y, z);
            g.cell[id] = 1;
        }
    }
    // Interior: everything the outside flood (6-connected) cannot reach.
    let total = g.cell.len();
    let mut outside = vec![false; total];
    let mut stack = vec![0usize];
    outside[0] = true;
    while let Some(i) = stack.pop() {
        let x = i % n[0];
        let y = (i / n[0]) % n[1];
        let z = i / (n[0] * n[1]);
        let mut push = |xx: isize, yy: isize, zz: isize| {
            if xx < 0 || yy < 0 || zz < 0 || xx as usize >= n[0] || yy as usize >= n[1] || zz as usize >= n[2] {
                return;
            }
            let j = (zz as usize * n[1] + yy as usize) * n[0] + xx as usize;
            if !outside[j] && g.cell[j] == 0 {
                outside[j] = true;
                stack.push(j);
            }
        };
        let (x, y, z) = (x as isize, y as isize, z as isize);
        push(x - 1, y, z);
        push(x + 1, y, z);
        push(x, y - 1, z);
        push(x, y + 1, z);
        push(x, y, z - 1);
        push(x, y, z + 1);
    }
    let shell = tri_shells(pos.len(), tris);
    let all = |_: usize| true;
    let ray_data = ray_analysis(&g, pos, tris, &shell, &all, compute_edges);
    let mut flooded = 0usize;
    for ((cell, &out), &vote) in g.cell.iter_mut().zip(&outside).zip(&ray_data.votes).take(total) {
        if *cell == 0 && (!out || vote >= 4) {
            if out {
                flooded += 1;
            }
            *cell = 2;
        }
    }
    // How sealed the body is: its own triangles only, so an open piece
    // (a cape is a sheet) is not counted as an opening.
    let is_body = |t: usize| body.is_none_or(|b| b[t]);
    let body_votes_owned;
    let body_votes: &[u8] = if body.is_some_and(|b| b.iter().any(|x| !x)) {
        body_votes_owned = ray_analysis(&g, pos, tris, &shell, &is_body, false).votes;
        &body_votes_owned
    } else {
        &ray_data.votes
    };
    let (mut inside, mut tied) = (0usize, 0usize);
    for i in 0..total {
        if g.cell[i] == 1 {
            continue;
        }
        if body_votes[i] >= 4 || (g.cell[i] == 2 && !outside[i]) {
            inside += 1;
        } else if body_votes[i] == 3 {
            tied += 1;
        }
    }
    let (n_open, largest) = openings(pos, tris, &is_body);
    let interior = (0..total).filter(|&i| g.cell[i] == 2).count();
    let contested = tied as f64 / (inside + tied).max(1) as f64;
    let scale = ext.max_elem().max(1e-9);
    let solid = Solid {
        openings: n_open,
        largest_opening: (largest / scale * 1e4).round() / 1e4,
        leaked: (flooded as f64 / interior.max(1) as f64 * 1e4).round() / 1e4,
        contested: (contested * 1e4).round() / 1e4,
        trusted: contested <= MAX_CONTESTED,
    };
    (g, solid, ray_data, shell)
}

pub fn voxelize(pos: &[Vec3], tris: &[[u32; 3]], res: usize) -> Grid {
    voxelize_body(pos, tris, res, None).0
}

/// `voxelize`, and how sealed the body is (`body[t]` marks the triangles
/// of the body, as against open pieces such as a cape; all when `None`).
pub fn voxelize_body(pos: &[Vec3], tris: &[[u32; 3]], res: usize, body: Option<&[bool]>) -> (Grid, Solid) {
    let (g, solid, _, _) = voxelize_internal(pos, tris, res, body, false);
    (g, solid)
}

/// Per-vertex nearest bones by cut-cell geodesic distance.
pub struct Geo {
    /// Up to K (joint, distance) per vertex, ascending distance.
    pub near: Vec<Vec<(u32, f32)>>,
    /// Distances at or beyond `cap` mean "unreachable / far".
    pub cap: f32,
    pub res: usize,
    /// How sealed the body is (its openings, and whether the fill can be
    /// trusted).
    pub solid: Solid,
}

pub const K: usize = 8;

impl Geo {
    /// Distance from vertex v to bone j; bones outside the top-K report the
    /// K-th distance (a lower bound) or the cap.
    pub fn dist(&self, v: usize, j: u32) -> f32 {
        let near = &self.near[v];
        if let Some(e) = near.iter().find(|e| e.0 == j) {
            return e.1;
        }
        if near.len() < K { self.cap } else { near.last().map(|e| e.1).unwrap_or(self.cap) }
    }
    pub fn nearest(&self, v: usize) -> Option<(u32, f32)> {
        self.near[v].first().copied()
    }
}

/// Scale-invariant open-segment vs triangle intersection returning `t in (1e-6, 1 - 1e-6)`,
/// conservatively inclusive on triangle edges so rays cannot slip through shared edges.
#[inline]
fn seg_tri_t(p0: Vec3, dir: Vec3, dir_len2: f64, a: Vec3, b: Vec3, c: Vec3) -> Option<f64> {
    let e1 = b - a;
    let e2 = c - a;
    let h = dir.cross(e2);
    let det = e1.dot(h);
    let scale2 = e1.len2() * e2.len2() * dir_len2;
    if det * det <= 1e-24 * scale2 {
        return None;
    }
    let inv = 1.0 / det;
    let s = p0 - a;
    let u = inv * s.dot(h);
    if !(-1e-7..=1.0 + 1e-7).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = inv * dir.dot(q);
    if v < -1e-7 || u + v > 1.0 + 1e-7 {
        return None;
    }
    let t = inv * e2.dot(q);
    (t > 1e-6 && t < 1.0 - 1e-6).then_some(t)
}

#[inline]
fn seg_hits_tri(p0: Vec3, dir: Vec3, dir_len2: f64, a: Vec3, b: Vec3, c: Vec3) -> bool {
    seg_tri_t(p0, dir, dir_len2, a, b, c).is_some()
}

/// Flat CSR grid mapping each voxel cell to the triangles touching its box.
struct TriGrid {
    n: [usize; 3],
    origin: Vec3,
    h: f64,
    offs: Vec<u32>,
    tris: Vec<u32>,
}

impl TriGrid {
    fn build(g: &Grid, pos: &[Vec3], tris: &[[u32; 3]]) -> Self {
        let total = g.cell.len();
        let n = g.n;
        let inv_h = 1.0 / g.h;
        let mut counts = vec![0u32; total];
        let each_cells = |visit: &mut dyn FnMut(usize, u32)| {
            for (ti, &t) in tris.iter().enumerate() {
                let (a, b, c) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
                let lo = a.min(b).min(c);
                let hi = a.max(b).max(c);
                let x0 = (((lo.x - g.origin.x) * inv_h - 1e-4).floor().max(0.0) as usize).min(n[0] - 1);
                let y0 = (((lo.y - g.origin.y) * inv_h - 1e-4).floor().max(0.0) as usize).min(n[1] - 1);
                let z0 = (((lo.z - g.origin.z) * inv_h - 1e-4).floor().max(0.0) as usize).min(n[2] - 1);
                let x1 = (((hi.x - g.origin.x) * inv_h + 1e-4).floor().max(0.0) as usize).min(n[0] - 1);
                let y1 = (((hi.y - g.origin.y) * inv_h + 1e-4).floor().max(0.0) as usize).min(n[1] - 1);
                let z1 = (((hi.z - g.origin.z) * inv_h + 1e-4).floor().max(0.0) as usize).min(n[2] - 1);
                let span = (x1 - x0 + 1) * (y1 - y0 + 1) * (z1 - z0 + 1);
                let plane = if span > 8 {
                    let nn = (b - a).cross(c - a);
                    let r = (nn.x.abs() + nn.y.abs() + nn.z.abs()) * (0.5005 * g.h);
                    Some((nn, r))
                } else {
                    None
                };
                for cz in z0..=z1 {
                    for cy in y0..=y1 {
                        let row = (cz * n[1] + cy) * n[0];
                        for cx in x0..=x1 {
                            if let Some((nn, r)) = plane {
                                let cc = g.center(cx, cy, cz);
                                if nn.dot(cc - a).abs() > r {
                                    continue;
                                }
                            }
                            visit(row + cx, ti as u32);
                        }
                    }
                }
            }
        };
        each_cells(&mut |cell, _| counts[cell] += 1);
        let mut offs = vec![0u32; total + 1];
        for i in 0..total {
            offs[i + 1] = offs[i] + counts[i];
        }
        let mut cursor = offs[..total].to_vec();
        let mut data = vec![0u32; offs[total] as usize];
        each_cells(&mut |cell, ti| {
            let p = cursor[cell] as usize;
            data[p] = ti;
            cursor[cell] += 1;
        });
        TriGrid { n, origin: g.origin, h: g.h, offs, tris: data }
    }

    #[inline]
    fn cell_empty(&self, idx: usize) -> bool {
        self.offs[idx] == self.offs[idx + 1]
    }

    #[inline]
    fn cell_of_clamped(&self, p: Vec3) -> [usize; 3] {
        let inv_h = 1.0 / self.h;
        [
            (((p.x - self.origin.x) * inv_h).floor().max(0.0) as usize).min(self.n[0] - 1),
            (((p.y - self.origin.y) * inv_h).floor().max(0.0) as usize).min(self.n[1] - 1),
            (((p.z - self.origin.z) * inv_h).floor().max(0.0) as usize).min(self.n[2] - 1),
        ]
    }

    #[allow(clippy::too_many_arguments)]
    fn seg_hits_in_cells(
        &self,
        p0: Vec3,
        p1: Vec3,
        lo: [usize; 3],
        hi: [usize; 3],
        pos: &[Vec3],
        tris: &[[u32; 3]],
        ev: Option<u32>,
    ) -> bool {
        let dir = p1 - p0;
        let dir_len2 = dir.len2();
        if dir_len2 < 1e-24 {
            return false;
        }
        let (nx, ny) = (self.n[0], self.n[1]);
        for cz in lo[2]..=hi[2] {
            for cy in lo[1]..=hi[1] {
                let row = (cz * ny + cy) * nx;
                for cx in lo[0]..=hi[0] {
                    let c = row + cx;
                    let s = self.offs[c] as usize;
                    let e = self.offs[c + 1] as usize;
                    for &ti in &self.tris[s..e] {
                        let t = tris[ti as usize];
                        if let Some(v) = ev
                            && (t[0] == v || t[1] == v || t[2] == v)
                        {
                            continue;
                        }
                        if seg_hits_tri(p0, dir, dir_len2, pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn seg_hits_any(&self, p0: Vec3, p1: Vec3, pos: &[Vec3], tris: &[[u32; 3]], exclude_vert: Option<u32>) -> bool {
        let inv_h = 1.0 / self.h;
        let lo = p0.min(p1);
        let hi = p0.max(p1);
        let c0 = [
            (((lo.x - self.origin.x) * inv_h - 1e-4).floor().max(0.0) as usize).min(self.n[0] - 1),
            (((lo.y - self.origin.y) * inv_h - 1e-4).floor().max(0.0) as usize).min(self.n[1] - 1),
            (((lo.z - self.origin.z) * inv_h - 1e-4).floor().max(0.0) as usize).min(self.n[2] - 1),
        ];
        let c1 = [
            (((hi.x - self.origin.x) * inv_h + 1e-4).floor().max(0.0) as usize).min(self.n[0] - 1),
            (((hi.y - self.origin.y) * inv_h + 1e-4).floor().max(0.0) as usize).min(self.n[1] - 1),
            (((hi.z - self.origin.z) * inv_h + 1e-4).floor().max(0.0) as usize).min(self.n[2] - 1),
        ];
        self.seg_hits_in_cells(p0, p1, c0, c1, pos, tris, exclude_vert)
    }

    /// True if the open segment `p0 -> p1` stays inside the solid union
    /// (allowing internal crossings of overlapping shells, rejecting any air gap).
    fn seg_stays_inside(&self, p0: Vec3, p1: Vec3, pos: &[Vec3], tris: &[[u32; 3]], shell: &[u32]) -> bool {
        let dir = p1 - p0;
        let dir_len2 = dir.len2();
        if dir_len2 < 1e-24 {
            return true;
        }
        let inv_h = 1.0 / self.h;
        let lo = p0.min(p1);
        let hi = p0.max(p1);
        let c0 = [
            (((lo.x - self.origin.x) * inv_h - 1e-4).floor().max(0.0) as usize).min(self.n[0] - 1),
            (((lo.y - self.origin.y) * inv_h - 1e-4).floor().max(0.0) as usize).min(self.n[1] - 1),
            (((lo.z - self.origin.z) * inv_h - 1e-4).floor().max(0.0) as usize).min(self.n[2] - 1),
        ];
        let c1 = [
            (((hi.x - self.origin.x) * inv_h + 1e-4).floor().max(0.0) as usize).min(self.n[0] - 1),
            (((hi.y - self.origin.y) * inv_h + 1e-4).floor().max(0.0) as usize).min(self.n[1] - 1),
            (((hi.z - self.origin.z) * inv_h + 1e-4).floor().max(0.0) as usize).min(self.n[2] - 1),
        ];
        let mut ts = Vec::new();
        let (nx, ny) = (self.n[0], self.n[1]);
        for cz in c0[2]..=c1[2] {
            for cy in c0[1]..=c1[1] {
                let row = (cz * ny + cy) * nx;
                for cx in c0[0]..=c1[0] {
                    let c = row + cx;
                    let s = self.offs[c] as usize;
                    let e = self.offs[c + 1] as usize;
                    for &ti in &self.tris[s..e] {
                        let t = tris[ti as usize];
                        if let Some(th) = seg_tri_t(p0, dir, dir_len2, pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]) {
                            ts.push(th);
                        }
                    }
                }
            }
        }
        if ts.is_empty() {
            return true;
        }
        ts.sort_by(|a, b| a.total_cmp(b));
        let mut prev = 0.0f64;
        for &th in &ts {
            if th - prev > 1e-5 {
                let mid = p0 + dir * (0.5 * (prev + th));
                if !self.point_inside_and_nearest_tris(mid, pos, tris, shell).0 {
                    return false;
                }
            }
            prev = th;
        }
        if 1.0 - prev > 1e-5 {
            let mid = p0 + dir * (0.5 * (prev + 1.0));
            if !self.point_inside_and_nearest_tris(mid, pos, tris, shell).0 {
                return false;
            }
        }
        true
    }

    /// Exact six-ray point-in-solid test at an arbitrary world point `p`, also
    /// returning the nearest enclosing hit triangles along the six rays (used
    /// for bone seeds inside thin parts narrower than a voxel cell).
    fn point_inside_and_nearest_tris(&self, p: Vec3, pos: &[Vec3], tris: &[[u32; 3]], shell: &[u32]) -> (bool, Vec<(usize, Vec3)>) {
        // Tiny sub-micron nudge keeps rays off exact mesh vertex grid lines.
        let pq = p + v3(1.37e-5, 2.91e-5, 4.19e-5) * self.h;
        let cell = self.cell_of_clamped(pq);
        let mut votes = 0u8;
        let mut nearest = Vec::new();
        let mut cand = Vec::new();
        let mut low_shells = Vec::new();
        let mut high_shells = Vec::new();

        for axis in 0..3 {
            let (ua, va) = ((axis + 1) % 3, (axis + 2) % 3);
            let (cu, cv) = (cell[ua], cell[va]);
            cand.clear();
            for k in 0..self.n[axis] {
                let mut q = [0usize; 3];
                q[axis] = k;
                q[ua] = cu;
                q[va] = cv;
                let idx = (q[2] * self.n[1] + q[1]) * self.n[0] + q[0];
                let s = self.offs[idx] as usize;
                let e = self.offs[idx + 1] as usize;
                cand.extend_from_slice(&self.tris[s..e]);
            }
            if cand.is_empty() {
                continue;
            }
            cand.sort_unstable();
            cand.dedup();
            low_shells.clear();
            high_shells.clear();
            let mut best_lo: Option<(f64, usize, f64)> = None;
            let mut best_hi: Option<(f64, usize, f64)> = None;
            let p2 = (pq[ua], pq[va]);
            let pa = pq[axis];

            for &ti in &cand {
                let t = tris[ti as usize];
                let (v0, v1, v2) = (pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]);
                let (a, b, c) = ((v0[ua], v0[va]), (v1[ua], v1[va]), (v2[ua], v2[va]));
                if let Some(depth) = ray_tri_2d(t, a, b, c, v0[axis], v1[axis], v2[axis], p2) {
                    let sh = shell[ti as usize];
                    if depth < pa {
                        low_shells.push(sh);
                        let dist = pa - depth;
                        if best_lo.is_none_or(|b| dist < b.0) {
                            best_lo = Some((dist, ti as usize, depth));
                        }
                    } else if depth > pa {
                        high_shells.push(sh);
                        let dist = depth - pa;
                        if best_hi.is_none_or(|b| dist < b.0) {
                            best_hi = Some((dist, ti as usize, depth));
                        }
                    }
                }
            }

            let has_odd = |shells: &mut Vec<u32>| -> bool {
                shells.sort_unstable();
                let mut i = 0;
                while i < shells.len() {
                    let mut j = i + 1;
                    while j < shells.len() && shells[j] == shells[i] {
                        j += 1;
                    }
                    if (j - i) & 1 == 1 {
                        return true;
                    }
                    i = j;
                }
                false
            };
            if has_odd(&mut low_shells) {
                votes += 1;
            }
            if has_odd(&mut high_shells) {
                votes += 1;
            }
            for hit in [best_lo, best_hi].into_iter().flatten() {
                let mut hp = [pq.x, pq.y, pq.z];
                hp[axis] = hit.2;
                nearest.push((hit.1, v3(hp[0], hp[1], hp[2])));
            }
        }
        (votes >= 4, nearest)
    }
}

/// Flat CSR grid mapping each voxel cell to the mesh vertices inside it.
struct VertGrid {
    n: [usize; 3],
    offs: Vec<u32>,
    verts: Vec<u32>,
}

impl VertGrid {
    fn build(g: &Grid, pos: &[Vec3]) -> Self {
        let total = g.cell.len();
        let n = g.n;
        let inv_h = 1.0 / g.h;
        let cell_of = |p: Vec3| {
            let cx = (((p.x - g.origin.x) * inv_h).floor().max(0.0) as usize).min(n[0] - 1);
            let cy = (((p.y - g.origin.y) * inv_h).floor().max(0.0) as usize).min(n[1] - 1);
            let cz = (((p.z - g.origin.z) * inv_h).floor().max(0.0) as usize).min(n[2] - 1);
            (cz * n[1] + cy) * n[0] + cx
        };
        let mut counts = vec![0u32; total];
        for &p in pos {
            counts[cell_of(p)] += 1;
        }
        let mut offs = vec![0u32; total + 1];
        for i in 0..total {
            offs[i + 1] = offs[i] + counts[i];
        }
        let mut cursor = offs[..total].to_vec();
        let mut verts = vec![0u32; offs[total] as usize];
        for (vi, &p) in pos.iter().enumerate() {
            let c = cell_of(p);
            let slot = cursor[c] as usize;
            verts[slot] = vi as u32;
            cursor[c] += 1;
        }
        VertGrid { n, offs, verts }
    }

    #[inline]
    fn cell_verts(&self, x: usize, y: usize, z: usize) -> &[u32] {
        let c = (z * self.n[1] + y) * self.n[0] + x;
        &self.verts[self.offs[c] as usize..self.offs[c + 1] as usize]
    }
}

/// Unified cut-cell geodesic graph:
/// - Nodes `0..n_int`: strictly interior grid nodes (`votes >= 4`) connected by
///   surface-cut 26-neighbor edges.
/// - Nodes `n_int..n_int + nv`: mesh vertices attached to unobstructed interior
///   nodes and connected along surface mesh edges (supporting thin parts).
struct CutCellGraph {
    n_int: usize,
    grid_to_node: Vec<u32>,
    int_pos: Vec<Vec3>,
    offs: Vec<u32>,
    dst: Vec<u32>,
    w: Vec<f32>,
    /// Unobstructed 2x2x2 interior corners `(node, dist_to_v, trilinear_weight)` per vertex.
    v_sample: Vec<Vec<(u32, f32, f64)>>,
}

impl CutCellGraph {
    fn build(g: &Grid, model: &Model, ray: &RayData, tri_grid: &TriGrid) -> Self {
        let (nx, ny, nz) = (g.n[0], g.n[1], g.n[2]);
        let total = g.cell.len();
        let nv = model.nverts();
        let mut grid_to_node = vec![u32::MAX; total];
        let mut int_pos = Vec::new();
        for z in 0..nz {
            for y in 0..ny {
                let row = (z * ny + y) * nx;
                for x in 0..nx {
                    let i = row + x;
                    if ray.votes[i] >= 4 {
                        grid_to_node[i] = int_pos.len() as u32;
                        int_pos.push(node_pos(g, x, y, z));
                    }
                }
            }
        }
        let n_int = int_pos.len();
        let n_nodes = n_int + nv;

        // 13 positive-lexicographic 26-neighbor offsets (undirected edges added once).
        let h = g.h as f32;
        let step_open = |a: [usize; 3], b: [usize; 3]| -> bool {
            let ia = g.idx(a[0], a[1], a[2]);
            let ib = g.idx(b[0], b[1], b[2]);
            if grid_to_node[ia] == u32::MAX || grid_to_node[ib] == u32::MAX {
                return false;
            }
            let (lo, axis) = if a[0] != b[0] {
                (if a[0] < b[0] { ia } else { ib }, 0u8)
            } else if a[1] != b[1] {
                (if a[1] < b[1] { ia } else { ib }, 1u8)
            } else {
                (if a[2] < b[2] { ia } else { ib }, 2u8)
            };
            (ray.axis_open[lo] & (1u8 << axis)) != 0
        };

        let axis_dirs: [([isize; 3], f32); 3] = [([1, 0, 0], h), ([0, 1, 0], h), ([0, 0, 1], h)];
        let face_dirs: [([isize; 3], f32); 6] = [
            ([1, 1, 0], h * std::f32::consts::SQRT_2),
            ([-1, 1, 0], h * std::f32::consts::SQRT_2),
            ([1, 0, 1], h * std::f32::consts::SQRT_2),
            ([-1, 0, 1], h * std::f32::consts::SQRT_2),
            ([0, 1, 1], h * std::f32::consts::SQRT_2),
            ([0, -1, 1], h * std::f32::consts::SQRT_2),
        ];
        let diag3_len = h * 3.0f32.sqrt();
        let space_dirs: [[isize; 3]; 4] = [[1, 1, 1], [-1, 1, 1], [1, -1, 1], [-1, -1, 1]];

        // Build interior cut-cell edges per Z slice in parallel (deterministic order).
        let slice_edges: Vec<Vec<(u32, u32, f32)>> = (0..nz)
            .into_par_iter()
            .map(|z| {
                let mut edges = Vec::new();
                for y in 0..ny {
                    for x in 0..nx {
                        let ia = g.idx(x, y, z);
                        let u = grid_to_node[ia];
                        if u == u32::MAX {
                            continue;
                        }
                        let a = [x, y, z];
                        let pa = int_pos[u as usize];

                        // Axis edges.
                        for &(d, len) in &axis_dirs {
                            let (xx, yy, zz) = (x as isize + d[0], y as isize + d[1], z as isize + d[2]);
                            if xx < 0 || yy < 0 || zz < 0 || xx as usize >= nx || yy as usize >= ny || zz as usize >= nz {
                                continue;
                            }
                            let b = [xx as usize, yy as usize, zz as usize];
                            if step_open(a, b) {
                                let v = grid_to_node[g.idx(b[0], b[1], b[2])];
                                edges.push((u, v, len));
                            }
                        }

                        // 2D face diagonals: require a local open L-path and zero triangle hits.
                        for &(d, len) in &face_dirs {
                            let (xx, yy, zz) = (x as isize + d[0], y as isize + d[1], z as isize + d[2]);
                            if xx < 0 || yy < 0 || zz < 0 || xx as usize >= nx || yy as usize >= ny || zz as usize >= nz {
                                continue;
                            }
                            let b = [xx as usize, yy as usize, zz as usize];
                            let v = grid_to_node[g.idx(b[0], b[1], b[2])];
                            if v == u32::MAX {
                                continue;
                            }
                            let (c1, c2) = if d[0] == 0 {
                                ([x, b[1], z], [x, y, b[2]])
                            } else if d[1] == 0 {
                                ([b[0], y, z], [x, y, b[2]])
                            } else {
                                ([b[0], y, z], [x, b[1], z])
                            };
                            let path_ok = (step_open(a, c1) && step_open(c1, b)) || (step_open(a, c2) && step_open(c2, b));
                            if !path_ok {
                                continue;
                            }
                            let lo = [x.min(b[0]), y.min(b[1]), z.min(b[2])];
                            let hi = [x.max(b[0]), y.max(b[1]), z.max(b[2])];
                            let empty = tri_grid.cell_empty(ia)
                                && tri_grid.cell_empty(g.idx(b[0], b[1], b[2]))
                                && tri_grid.cell_empty(g.idx(c1[0], c1[1], c1[2]))
                                && tri_grid.cell_empty(g.idx(c2[0], c2[1], c2[2]));
                            let pb = int_pos[v as usize];
                            if empty || !tri_grid.seg_hits_in_cells(pa, pb, lo, hi, &model.rest, &model.tris, None) {
                                edges.push((u, v, len));
                            }
                        }

                        // 3D space diagonals: require a local open 3-step axis path and zero triangle hits.
                        for &d in &space_dirs {
                            let (xx, yy, zz) = (x as isize + d[0], y as isize + d[1], z as isize + d[2]);
                            if xx < 0 || yy < 0 || zz < 0 || xx as usize >= nx || yy as usize >= ny || zz as usize >= nz {
                                continue;
                            }
                            let b = [xx as usize, yy as usize, zz as usize];
                            let v = grid_to_node[g.idx(b[0], b[1], b[2])];
                            if v == u32::MAX {
                                continue;
                            }
                            let ax = [b[0], y, z];
                            let ay = [x, b[1], z];
                            let az = [x, y, b[2]];
                            let bxy = [b[0], b[1], z];
                            let bxz = [b[0], y, b[2]];
                            let byz = [x, b[1], b[2]];
                            let path_ok = (step_open(a, ax)
                                && ((step_open(ax, bxy) && step_open(bxy, b)) || (step_open(ax, bxz) && step_open(bxz, b))))
                                || (step_open(a, ay)
                                    && ((step_open(ay, bxy) && step_open(bxy, b)) || (step_open(ay, byz) && step_open(byz, b))))
                                || (step_open(a, az)
                                    && ((step_open(az, bxz) && step_open(bxz, b)) || (step_open(az, byz) && step_open(byz, b))));
                            if !path_ok {
                                continue;
                            }
                            let lo = [x.min(b[0]), y.min(b[1]), z.min(b[2])];
                            let hi = [x.max(b[0]), y.max(b[1]), z.max(b[2])];
                            let mut empty = true;
                            'box_check: for cz in lo[2]..=hi[2] {
                                for cy in lo[1]..=hi[1] {
                                    for cx in lo[0]..=hi[0] {
                                        if !tri_grid.cell_empty(g.idx(cx, cy, cz)) {
                                            empty = false;
                                            break 'box_check;
                                        }
                                    }
                                }
                            }
                            let pb = int_pos[v as usize];
                            if empty || !tri_grid.seg_hits_in_cells(pa, pb, lo, hi, &model.rest, &model.tris, None) {
                                edges.push((u, v, diag3_len));
                            }
                        }
                    }
                }
                edges
            })
            .collect();

        type VertAttach = (Vec<(u32, f32)>, Vec<(u32, f32, f64)>);
        // Side-aware vertex attachments to unobstructed interior grid nodes.
        let v_attach: Vec<VertAttach> = (0..nv)
            .into_par_iter()
            .map(|v| {
                let p = model.rest[v];
                let raw = g.coord(p);
                let c = [raw[0] - RAY_EPS[0], raw[1] - RAY_EPS[1], raw[2] - RAY_EPS[2]];
                let base = [c[0].floor() as isize, c[1].floor() as isize, c[2].floor() as isize];
                let f = [c[0] - base[0] as f64, c[1] - base[1] as f64, c[2] - base[2] as f64];
                let mut edges = Vec::new();
                let mut samples = Vec::new();

                for k in 0..8 {
                    let o = [k & 1, (k >> 1) & 1, (k >> 2) & 1];
                    let ix = base[0] + o[0] as isize;
                    let iy = base[1] + o[1] as isize;
                    let iz = base[2] + o[2] as isize;
                    if ix < 0 || iy < 0 || iz < 0 || ix as usize >= nx || iy as usize >= ny || iz as usize >= nz {
                        continue;
                    }
                    let node = grid_to_node[g.idx(ix as usize, iy as usize, iz as usize)];
                    if node == u32::MAX {
                        continue;
                    }
                    let q = int_pos[node as usize];
                    if !tri_grid.seg_hits_any(p, q, &model.rest, &model.tris, Some(v as u32)) {
                        let len = (q - p).len() as f32;
                        let w = (if o[0] == 1 { f[0] } else { 1.0 - f[0] })
                            * (if o[1] == 1 { f[1] } else { 1.0 - f[1] })
                            * (if o[2] == 1 { f[2] } else { 1.0 - f[2] });
                        edges.push((node, len));
                        samples.push((node, len, w.max(1e-6)));
                    }
                }

                // Fallback for corners/ridges with no unobstructed 2x2x2 interior corner:
                // check 3x3x3 neighborhood (still strictly side-aware via segment test).
                if edges.is_empty() && n_int > 0 {
                    let rc = [c[0].round() as isize, c[1].round() as isize, c[2].round() as isize];
                    for dz in -1isize..=1 {
                        for dy in -1isize..=1 {
                            for dx in -1isize..=1 {
                                let (ix, iy, iz) = (rc[0] + dx, rc[1] + dy, rc[2] + dz);
                                if ix < 0 || iy < 0 || iz < 0 || ix as usize >= nx || iy as usize >= ny || iz as usize >= nz {
                                    continue;
                                }
                                let node = grid_to_node[g.idx(ix as usize, iy as usize, iz as usize)];
                                if node == u32::MAX {
                                    continue;
                                }
                                let q = int_pos[node as usize];
                                if !tri_grid.seg_hits_any(p, q, &model.rest, &model.tris, Some(v as u32)) {
                                    edges.push((node, (q - p).len() as f32));
                                }
                            }
                        }
                    }
                }
                (edges, samples)
            })
            .collect();

        // Connect interior volumetric pockets across any surface edge whose endpoints'
        // unobstructed 2x2x2 interior samples fall in disjoint volumetric components.
        fn find_root(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        let mut parent: Vec<usize> = (0..n_int).collect();
        for list in &slice_edges {
            for &(u, v, _) in list {
                let ru = find_root(&mut parent, u as usize);
                let rv = find_root(&mut parent, v as usize);
                if ru != rv {
                    parent[rv] = ru;
                }
            }
        }
        let int_comp: Vec<usize> = (0..n_int).map(|i| find_root(&mut parent, i)).collect();

        // Thin-part vertices (no 2x2x2 interior corner), vertices bridging severed
        // volumetric pockets, and their junction neighbors enter the surface graph;
        // thick body regions stay 100% pure volumetric grid (density-invariant).
        let has_2x2x2: Vec<bool> = v_attach.iter().map(|x| !x.1.is_empty()).collect();
        let bridge_edges: Vec<[u32; 2]> = model
            .edges
            .iter()
            .copied()
            .filter(|&[u, v]| {
                let (ui, vi) = (u as usize, v as usize);
                if !has_2x2x2[ui] || !has_2x2x2[vi] {
                    return true;
                }
                let su = &v_attach[ui].1;
                let sv = &v_attach[vi].1;
                !su.iter().any(|&(nu, _, _)| sv.iter().any(|&(nv_node, _, _)| int_comp[nu as usize] == int_comp[nv_node as usize]))
            })
            .collect();
        let mut need_graph: Vec<bool> = has_2x2x2.iter().map(|&h| !h).collect();
        for &[u, v] in &bridge_edges {
            need_graph[u as usize] = true;
            need_graph[v as usize] = true;
        }

        // Assemble deterministic CSR adjacency.
        let mut deg = vec![0u32; n_nodes];
        for list in &slice_edges {
            for &(u, v, _) in list {
                deg[u as usize] += 1;
                deg[v as usize] += 1;
            }
        }
        for (v, (edges, _)) in v_attach.iter().enumerate() {
            if !need_graph[v] {
                continue;
            }
            let vn = n_int + v;
            deg[vn] += edges.len() as u32;
            for &(u, _) in edges {
                deg[u as usize] += 1;
            }
        }
        for &[u, v] in &bridge_edges {
            deg[n_int + u as usize] += 1;
            deg[n_int + v as usize] += 1;
        }
        let mut offs = vec![0u32; n_nodes + 1];
        for i in 0..n_nodes {
            offs[i + 1] = offs[i] + deg[i];
        }
        let mut cursor = offs[..n_nodes].to_vec();
        let m = offs[n_nodes] as usize;
        let mut dst = vec![0u32; m];
        let mut w = vec![0.0f32; m];
        let mut push_edge = |u: usize, v: usize, len: f32| {
            let su = cursor[u] as usize;
            dst[su] = v as u32;
            w[su] = len;
            cursor[u] += 1;
            let sv = cursor[v] as usize;
            dst[sv] = u as u32;
            w[sv] = len;
            cursor[v] += 1;
        };
        for list in &slice_edges {
            for &(u, v, len) in list {
                push_edge(u as usize, v as usize, len);
            }
        }
        let mut v_sample = Vec::with_capacity(nv);
        for (v, (edges, samples)) in v_attach.into_iter().enumerate() {
            if need_graph[v] {
                let vn = n_int + v;
                for (u, len) in edges {
                    push_edge(vn, u as usize, len);
                }
            }
            v_sample.push(samples);
        }
        for &[u, v] in &bridge_edges {
            let len = (model.rest[u as usize] - model.rest[v as usize]).len() as f32;
            push_edge(n_int + u as usize, n_int + v as usize, len);
        }
        CutCellGraph { n_int, grid_to_node, int_pos, offs, dst, w, v_sample }
    }
}

fn push_vert_seed(graph: &CutCellGraph, seeds: &mut Vec<(usize, f32)>, vi: usize, d: f32) {
    let vn = graph.n_int + vi;
    let mut added = false;
    if graph.offs[vn] < graph.offs[vn + 1] {
        seeds.push((vn, d));
        added = true;
    }
    for &(node, len, _) in &graph.v_sample[vi] {
        seeds.push((node as usize, d + len));
        added = true;
    }
    if !added {
        seeds.push((vn, d));
    }
}

#[allow(clippy::too_many_arguments)]
fn bone_seeds(
    g: &Grid,
    graph: &CutCellGraph,
    tri_grid: &TriGrid,
    vert_grid: &VertGrid,
    pos: &[Vec3],
    tris: &[[u32; 3]],
    shell: &[u32],
    a: Vec3,
    b: Vec3,
) -> Vec<(usize, f32)> {
    let (nx, ny, nz) = (g.n[0], g.n[1], g.n[2]);
    let len = (b - a).len();
    let steps = ((len / (g.h * 0.5)).ceil() as usize).max(1);
    let mut seeds: Vec<(usize, f32)> = Vec::new();
    let r2_thin = (1.5 * g.h).powi(2);

    let mut was_inside = false;
    let mut prev_p = a;
    for s in 0..=steps {
        let p = a.lerp(b, s as f64 / steps as f64);
        // If an estimated bone segment exits the solid into air (e.g. a spine
        // bone pointing diagonally to a shoulder child across an armpit gap),
        // stop seeding so the bone never jumps into another limb.
        if s > 0 && was_inside && !tris.is_empty() && !tri_grid.seg_stays_inside(prev_p, p, pos, tris, shell) {
            break;
        }
        prev_p = p;
        let raw = g.coord(p);
        let c = [raw[0] - RAY_EPS[0], raw[1] - RAY_EPS[1], raw[2] - RAY_EPS[2]];
        let base = [c[0].floor() as isize, c[1].floor() as isize, c[2].floor() as isize];
        let mut found_int = false;
        for k in 0..8 {
            let ix = base[0] + (k & 1) as isize;
            let iy = base[1] + ((k >> 1) & 1) as isize;
            let iz = base[2] + ((k >> 2) & 1) as isize;
            if ix < 0 || iy < 0 || iz < 0 || ix as usize >= nx || iy as usize >= ny || iz as usize >= nz {
                continue;
            }
            let node = graph.grid_to_node[g.idx(ix as usize, iy as usize, iz as usize)];
            if node == u32::MAX {
                continue;
            }
            let q = graph.int_pos[node as usize];
            if !tri_grid.seg_hits_any(p, q, pos, tris, None) {
                let d = segment_distance(q, a, b).1 as f32;
                seeds.push((node as usize, d));
                found_int = true;
            }
        }
        if found_int {
            was_inside = true;
        }

        // Thin-part support: if no interior grid node is unobstructed from `p`,
        // check whether `p` itself lies inside a thin solid (< h thick) and seed
        // its enclosing surface vertices without crossing air gaps.
        if !found_int && !tris.is_empty() {
            let (inside, nearest_tris) = tri_grid.point_inside_and_nearest_tris(p, pos, tris, shell);
            if inside {
                was_inside = true;
                for (ti, hit_pos) in nearest_tris {
                    let dp = (p - hit_pos).len();
                    if dp <= 2.0 * g.h {
                        for &vi in &tris[ti] {
                            let d = (dp + (pos[vi as usize] - hit_pos).len()) as f32;
                            push_vert_seed(graph, &mut seeds, vi as usize, d);
                        }
                    }
                }
                let cell = tri_grid.cell_of_clamped(p);
                for dz in -1isize..=1 {
                    let cz = cell[2] as isize + dz;
                    if cz < 0 || cz as usize >= nz {
                        continue;
                    }
                    for dy in -1isize..=1 {
                        let cy = cell[1] as isize + dy;
                        if cy < 0 || cy as usize >= ny {
                            continue;
                        }
                        for dx in -1isize..=1 {
                            let cx = cell[0] as isize + dx;
                            if cx < 0 || cx as usize >= nx {
                                continue;
                            }
                            for &vi in vert_grid.cell_verts(cx as usize, cy as usize, cz as usize) {
                                let pv = pos[vi as usize];
                                if (pv - p).len2() <= r2_thin && !tri_grid.seg_hits_any(p, pv, pos, tris, Some(vi)) {
                                    let d = segment_distance(pv, a, b).1 as f32;
                                    push_vert_seed(graph, &mut seeds, vi as usize, d);
                                }
                            }
                        }
                    }
                }
            } else if was_inside {
                break;
            }
        }
    }

    if seeds.is_empty() {
        // Bone outside the mesh: seed the nearest interior node or mesh vertex.
        let mut best = (f64::MAX, usize::MAX);
        for (i, &q) in graph.int_pos.iter().enumerate() {
            let d = segment_distance(q, a, b).1;
            if d < best.0 {
                best = (d, i);
            }
        }
        for (v, &pv) in pos.iter().enumerate() {
            let d = segment_distance(pv, a, b).1;
            if d < best.0 {
                best = (d, graph.n_int + v);
            }
        }
        if best.1 != usize::MAX {
            if best.1 >= graph.n_int {
                push_vert_seed(graph, &mut seeds, best.1 - graph.n_int, best.0 as f32);
            } else {
                seeds.push((best.1, best.0 as f32));
            }
        }
    }

    // Keep the smallest seed distance per node deterministically.
    seeds.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
    seeds.dedup_by_key(|s| s.0);
    seeds
}

fn dijkstra(graph: &CutCellGraph, seeds: &[(usize, f32)], cap: f32) -> Vec<f32> {
    let n_nodes = graph.offs.len() - 1;
    let mut dist = vec![f32::INFINITY; n_nodes];
    let mut heap = BinaryHeap::new();
    for &(i, raw_d) in seeds {
        let d = if raw_d <= 0.0 { 0.0 } else { raw_d };
        if d < dist[i] {
            dist[i] = d;
            heap.push(Reverse((d.to_bits(), i as u32)));
        }
    }
    while let Some(Reverse((db, i))) = heap.pop() {
        let d = f32::from_bits(db);
        let i = i as usize;
        if d > dist[i] || d > cap {
            continue;
        }
        let s = graph.offs[i] as usize;
        let e = graph.offs[i + 1] as usize;
        for k in s..e {
            let j = graph.dst[k] as usize;
            let nd = d + graph.w[k];
            if nd < dist[j] {
                dist[j] = nd;
                heap.push(Reverse((nd.to_bits(), j as u32)));
            }
        }
    }
    dist
}

/// Side-aware distance at vertex `v`: smooth trilinear blend of `dist[q] + |p_v - q|`
/// over unobstructed 2x2x2 interior corners, combined with the thin-part graph distance.
#[inline]
fn sample_vertex(graph: &CutCellGraph, dist: &[f32], v: usize, cap: f32) -> f32 {
    let d_graph = dist[graph.n_int + v];
    let samples = &graph.v_sample[v];
    let d_smooth = if !samples.is_empty() {
        let (mut acc, mut wsum) = (0.0f64, 0.0f64);
        for &(node, len, w) in samples {
            let dn = dist[node as usize];
            if dn.is_finite() {
                acc += (dn + len) as f64 * w;
                wsum += w;
            }
        }
        if wsum > 0.0 { (acc / wsum) as f32 } else { f32::INFINITY }
    } else {
        f32::INFINITY
    };
    d_graph.min(d_smooth).min(cap)
}

/// Geodesic distances from every joint in `joints` to every vertex.
pub fn geodesics(model: &Model, res: usize, joints: &[usize]) -> Geo {
    let body: Vec<bool> = model.tris.iter().map(|t| !model.parts[model.vpart[t[0] as usize] as usize].piece).collect();
    let (g, solid, ray_data, shell) = voxelize_internal(&model.rest, &model.tris, res, Some(&body), true);
    let tri_grid = TriGrid::build(&g, &model.rest, &model.tris);
    let vert_grid = VertGrid::build(&g, &model.rest);
    let graph = CutCellGraph::build(&g, model, &ray_data, &tri_grid);
    let cap = (model.scale * 0.75) as f32;
    let nv = model.nverts();
    let mut near: Vec<Vec<(u32, f32)>> = vec![Vec::new(); nv];
    for chunk in joints.chunks(16) {
        let cols: Vec<(u32, Vec<f32>)> = chunk
            .par_iter()
            .map(|&j| {
                let seeds =
                    bone_seeds(&g, &graph, &tri_grid, &vert_grid, &model.rest, &model.tris, &shell, model.skel.head[j], model.skel.tail[j]);
                let dist = dijkstra(&graph, &seeds, cap);
                let col: Vec<f32> = (0..nv).map(|v| sample_vertex(&graph, &dist, v, cap)).collect();
                (j as u32, col)
            })
            .collect();
        near.par_iter_mut().enumerate().for_each(|(v, list)| {
            for (j, col) in &cols {
                list.push((*j, col[v]));
            }
            list.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            list.truncate(K);
        });
    }
    Geo { near, cap, res, solid }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{Bone, Fixture, to_glb};

    fn grid_vid(
        vmap: &mut std::collections::BTreeMap<(usize, usize, usize), u32>,
        pos: &mut Vec<Vec3>,
        lo: f64,
        step: f64,
        ix: usize,
        iy: usize,
        iz: usize,
    ) -> u32 {
        *vmap.entry((ix, iy, iz)).or_insert_with(|| {
            let id = pos.len() as u32;
            pos.push(v3(lo + ix as f64 * step, lo + iy as f64 * step, lo + iz as f64 * step));
            id
        })
    }

    fn add_outward_quad(pos: &[Vec3], tris: &mut Vec<[u32; 3]>, a: u32, b: u32, c: u32, d: u32) {
        let ctr = (pos[a as usize] + pos[b as usize] + pos[c as usize] + pos[d as usize]) * 0.25;
        let nrm = (pos[b as usize] - pos[a as usize]).cross(pos[c as usize] - pos[a as usize]);
        if nrm.dot(ctr) >= 0.0 {
            tris.push([a, b, c]);
            tris.push([a, c, d]);
        } else {
            tris.push([a, c, b]);
            tris.push([a, d, c]);
        }
    }

    #[test]
    fn outside_bone_seed_on_degree0_surface_vertex_reaches_interior() {
        // Subdivided watertight cube [-0.3, 0.3]^3 so surface vertices with valid 2x2x2 interior
        // corners have `need_graph == false` (degree-0 `n_int + v` nodes) and rely on `v_sample`.
        let n = 4usize;
        let lo = -0.3f64;
        let step = 0.6 / n as f64;
        let mut pos = Vec::<Vec3>::new();
        let mut vmap = std::collections::BTreeMap::<(usize, usize, usize), u32>::new();
        let mut tris = Vec::<[u32; 3]>::new();
        for j in 0..n {
            for i in 0..n {
                for &(k, x_face, y_face) in
                    &[(0usize, false, false), (n, false, false), (0, true, false), (n, true, false), (0, false, true), (n, false, true)]
                {
                    let (a, b, c, d) = if x_face {
                        (
                            grid_vid(&mut vmap, &mut pos, lo, step, i, k, j),
                            grid_vid(&mut vmap, &mut pos, lo, step, i + 1, k, j),
                            grid_vid(&mut vmap, &mut pos, lo, step, i + 1, k, j + 1),
                            grid_vid(&mut vmap, &mut pos, lo, step, i, k, j + 1),
                        )
                    } else if y_face {
                        (
                            grid_vid(&mut vmap, &mut pos, lo, step, k, i, j),
                            grid_vid(&mut vmap, &mut pos, lo, step, k, i + 1, j),
                            grid_vid(&mut vmap, &mut pos, lo, step, k, i + 1, j + 1),
                            grid_vid(&mut vmap, &mut pos, lo, step, k, i, j + 1),
                        )
                    } else {
                        (
                            grid_vid(&mut vmap, &mut pos, lo, step, i, j, k),
                            grid_vid(&mut vmap, &mut pos, lo, step, i + 1, j, k),
                            grid_vid(&mut vmap, &mut pos, lo, step, i + 1, j + 1, k),
                            grid_vid(&mut vmap, &mut pos, lo, step, i, j + 1, k),
                        )
                    };
                    add_outward_quad(&pos, &mut tris, a, b, c, d);
                }
            }
        }
        let bones = vec![
            Bone { name: "root", parent: None, head: v3(0.0, 0.0, 0.32), tail: v3(0.0, 0.0, 0.38) },
            Bone { name: "tip", parent: Some(0), head: v3(0.0, 0.0, 0.38), tail: v3(0.0, 0.0, 0.42) },
        ];
        let body_w = vec![vec![(0u32, 1.0f64)]; pos.len()];
        let f = Fixture { bones, body_pos: pos, body_tris: tris, cape_pos: Vec::new(), cape_tris: Vec::new(), body_w, cape_w: Vec::new() };
        let m = Model::from_glb(to_glb(&f)).unwrap();
        let center_v = m.rest.iter().position(|p| p.x.abs() < 1e-6 && p.y.abs() < 1e-6 && (p.z - 0.3).abs() < 1e-6).unwrap();

        let (g, _, ray_data, shell) = voxelize_internal(&m.rest, &m.tris, 16, None, true);
        let tri_grid = TriGrid::build(&g, &m.rest, &m.tris);
        let vert_grid = VertGrid::build(&g, &m.rest);
        let graph = CutCellGraph::build(&g, &m, &ray_data, &tri_grid);
        let vn = graph.n_int + center_v;
        assert_eq!(graph.offs[vn], graph.offs[vn + 1], "face-center vertex should be degree-0 in vertex graph");
        assert!(!graph.v_sample[center_v].is_empty(), "face-center vertex must have unobstructed 2x2x2 samples");
        let seeds = bone_seeds(&g, &graph, &tri_grid, &vert_grid, &m.rest, &m.tris, &shell, m.skel.head[0], m.skel.tail[0]);
        assert!(seeds.iter().any(|&(node, _)| node < graph.n_int), "push_vert_seed must seed interior nodes");

        let geo = geodesics(&m, 16, &[0]);
        let mut reachable = 0usize;
        for (v, p) in m.rest.iter().enumerate() {
            if p.z > 0.29 && v != center_v {
                let d = geo.dist(v, 0);
                assert!(d < geo.cap * 0.9, "neighbor {v} at {p:?} stayed unreachable (d={d}, cap={})", geo.cap);
                reachable += 1;
            }
        }
        assert!(reachable >= 8);
    }

    #[test]
    fn voxelize_empty_and_non_finite_mesh_does_not_panic() {
        let (g, solid, _, _) = voxelize_internal(&[], &[], 16, None, false);
        assert!(g.n[0] >= 1 && g.n[1] >= 1 && g.n[2] >= 1);
        assert_eq!(solid.openings, 0);

        let bad_pos = [v3(f64::NAN, 0.0, 0.0), v3(1e308, -1e308, 0.0)];
        let (g2, _, _, _) = voxelize_internal(&bad_pos, &[[0, 1, 99]], 32, None, false);
        assert!(g2.n[0] >= 1 && g2.h.is_finite());
    }
}
