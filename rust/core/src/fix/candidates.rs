use crate::math::Vec3;
use crate::metrics::{Ctx, Eval};
use crate::scene::{Model, VW, Weights, normalize_vw, prune_vw, sort_vw, weight_of};
use crate::skin::dqs;
use crate::transfer::{TriIndex, inpaint_from_matches, match_subset};
use rayon::prelude::*;
use std::collections::BTreeMap;

use super::helpers::{deform_bones, dilate, grow, is_piece, passes, pieces_from_body, prune_all, smooth, widths};
use super::picking::l1;

// --------------------------------------------------------------- candidates

/// Method 1, clean-up: drop bleed influences (bones far through the body),
/// treat speckled vertices as unknown and inpaint them from their clean
/// neighbors (a median-like repair that does not smear noise around),
/// smooth only where the check saw tears or collapse, prune to 4.
pub fn cleanup(model: &Model, ctx: &Ctx, ev: &Eval, smooth_tears: bool) -> Weights {
    use crate::metrics::{F_INTERSECT, F_NOISE};
    let th = &ctx.th;
    let deform = deform_bones(model);
    let w: Weights = (0..model.nverts())
        .into_par_iter()
        .map(|v| {
            let vw = &model.weights[v];
            let Some(near) = ctx.geo.nearest(v) else { return vw.clone() };
            let kept: VW =
                vw.iter().copied().filter(|&(j, _)| ((ctx.geo.dist(v, j) - near.1) as f64) <= th.bleed_excess * model.scale).collect();
            // All of it bleed: the nearest bone that deforms (an unweighted
            // end or IK bone can be nearest; review 2026-11-13).
            let fallback = ctx.geo.near[v].iter().find(|e| deform[e.0 as usize]).map(|e| e.0).unwrap_or(near.0);
            let kept = if kept.is_empty() { vec![(fallback, 1.0)] } else { kept };
            prune_vw(&normalize_vw(kept), 4, 0.01)
        })
        .collect();
    // Speckle is impulse noise: inside noisy areas, a vertex far from the
    // componentwise median of its neighbors takes that median (two passes,
    // so a speckle next to a speckle is caught too). Clean vertices, steep
    // falloffs, and hard steps all sit on their neighbors' median.
    // Seeds: the speckle that shows, and in a region where some does, the
    // speckle that does not show yet (it shows once its neighbours are
    // cleaned).
    let mut shows_in = vec![false; ctx.regions.len()];
    for v in 0..model.nverts() {
        if ev.flags[v] & F_NOISE != 0 {
            shows_in[ctx.vregion[v] as usize] = true;
        }
    }
    let seeds: Vec<bool> = (0..model.nverts())
        .map(|v| ev.flags[v] & F_NOISE != 0 || (shows_in[ctx.vregion[v] as usize] && ev.speckle.get(v).is_some_and(|&x| x > th.noise)))
        .collect();
    let noisy_area = dilate(model, &seeds, 2);
    // Worst outliers first and in sequence, so each median sees neighbors
    // that are already cleaned (a plain parallel median breaks down when
    // half the area is speckled).
    let median_of = |w: &Weights, v: usize| -> VW {
        let n = &model.adj[v];
        let mut bones: Vec<u32> = n.iter().flat_map(|&u| w[u as usize].iter().map(|e| e.0)).collect();
        bones.sort_unstable();
        bones.dedup();
        normalize_vw(
            bones
                .iter()
                .map(|&j| {
                    let mut xs: Vec<f64> = n.iter().map(|&u| weight_of(&w[u as usize], j)).collect();
                    xs.sort_by(|a, b| a.total_cmp(b));
                    let k = xs.len();
                    (j, if k % 2 == 1 { xs[k / 2] } else { 0.5 * (xs[k / 2 - 1] + xs[k / 2]) })
                })
                .collect(),
        )
    };
    let mut w = w;
    let thr = 0.4 * th.noise;
    for _ in 0..3 {
        let mut order: Vec<(f64, usize)> = (0..model.nverts())
            .filter(|&v| noisy_area[v] && model.adj[v].len() >= 3)
            .map(|v| (l1(&w[v], &median_of(&w, v)), v))
            .filter(|x| x.0 > thr)
            .collect();
        if order.is_empty() {
            break;
        }
        order.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, v) in order {
            let med = median_of(&w, v);
            if !med.is_empty() && l1(&w[v], &med) > thr {
                w[v] = prune_vw(&med, 4, 0.01);
            }
        }
    }
    if !smooth_tears {
        return prune_all(&w);
    }
    let torn: Vec<bool> = ev.flags.iter().map(|&f| f & !(F_INTERSECT | F_NOISE) != 0).collect();
    let mask = grow(model, &torn, widths::TORN_MARGIN * ctx.feature, None);
    let n = passes(model, &mask, widths::TORN_SMOOTH * ctx.feature);
    prune_all(&smooth(model, &w, &mask, n, 0.5))
}

/// Method 2, geodesic voxel binding (Dionne & de Lasa 2013): weight falls
/// off with geodesic distance through the voxelized body,
/// `w = (1 / ((1 - a) d + a d^2))^2`, top 4, then light smoothing.
/// Pieces then get robust transfer from the new body weights.
pub fn geodesic(model: &Model, ctx: &Ctx) -> Weights {
    let deform = deform_bones(model);
    let alpha = 0.5;
    let w: Weights = (0..model.nverts())
        .into_par_iter()
        .map(|v| {
            let mut vw: VW = ctx.geo.near[v]
                .iter()
                .filter(|e| deform[e.0 as usize] && e.1 < ctx.geo.cap)
                .map(|&(j, d)| {
                    let d = (d as f64 / model.scale).max(1e-4);
                    (j, (1.0 / ((1.0 - alpha) * d + alpha * d * d)).powi(2))
                })
                .collect();
            if vw.is_empty() {
                // Unreachable through the voxels: nearest bone segment.
                let j = (0..model.njoints())
                    .filter(|&j| deform[j])
                    .min_by(|&a, &b| model.skel.seg_dist(a, model.rest[v]).total_cmp(&model.skel.seg_dist(b, model.rest[v])))
                    .unwrap_or(0);
                vw.push((j as u32, 1.0));
            }
            prune_vw(&normalize_vw(vw), 4, 0.01)
        })
        .collect();
    let body: Vec<bool> = (0..model.nverts()).map(|v| !is_piece(model, v)).collect();
    let n = passes(model, &body, widths::GEODESIC_SMOOTH * ctx.feature);
    let w = prune_all(&smooth(model, &w, &body, n, 0.5));
    prune_all(&pieces_from_body(model, ctx, &w))
}

/// Method 2b, joint bands: each body vertex belongs to its geodesically
/// nearest bone, and blends with the connected parent or child bone over
/// a band one limb radius wide on each side of the joint (smoothstep).
/// Restores a standard falloff where the input's is missing, too hard, or
/// far too wide. Pieces get robust transfer from the new body weights.
pub fn joint_bands(model: &Model, ctx: &Ctx) -> Weights {
    let sk = &model.skel;
    let deform = deform_bones(model);
    let nj = model.njoints();
    let nearest = |v: usize| -> Option<u32> { ctx.geo.near[v].iter().find(|e| deform[e.0 as usize] && e.1 < ctx.geo.cap).map(|e| e.0) };
    // Median limb radius per bone.
    let mut dists: Vec<Vec<f64>> = vec![Vec::new(); nj];
    for v in 0..model.nverts() {
        if !is_piece(model, v)
            && let Some(j) = nearest(v)
        {
            dists[j as usize].push(sk.seg_dist(j as usize, model.rest[v]));
        }
    }
    let radius: Vec<f64> = dists
        .into_iter()
        .map(|mut d| {
            if d.is_empty() {
                return 0.0;
            }
            d.sort_by(|a, b| a.total_cmp(b));
            d[d.len() / 2]
        })
        .collect();
    let connected = |child: usize| -> Option<usize> {
        let p = sk.jparent[child]?;
        (deform[p] && (sk.head[child] - sk.tail[p]).len() < 0.02 * model.scale).then_some(p)
    };
    let smoothstep = |x: f64| {
        let t = x.clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let w: Weights = (0..model.nverts())
        .into_par_iter()
        .map(|v| {
            if is_piece(model, v) {
                return model.weights[v].clone();
            }
            let Some(j) = nearest(v) else { return model.weights[v].clone() };
            let j = j as usize;
            let x = model.rest[v];
            let dir = (sk.tail[j] - sk.head[j]).normalized();
            // Joint at our head (with parent)?
            // The band of the nearer end: on a bone shorter than its band
            // (a spine segment), the head band used to take every vertex,
            // leaving a step at the child (review 2026-11-13).
            let len = (sk.tail[j] - sk.head[j]).len();
            let has_child = sk.jchildren[j].iter().any(|&c| deform[c] && connected(c) == Some(j));
            if let Some(p) = connected(j) {
                let bw = radius[j].max(radius[p]).max(1e-6);
                let s = (x - sk.head[j]).dot(dir);
                if s < bw && (!has_child || s < 0.5 * len) {
                    let t = smoothstep((s + bw) / (2.0 * bw));
                    return normalize_vw(vec![(j as u32, t), (p as u32, 1.0 - t)]);
                }
            }
            // Joint at our tail (with the nearest connected child)?
            let child = sk.jchildren[j]
                .iter()
                .copied()
                .filter(|&c| deform[c] && connected(c) == Some(j))
                .min_by(|&a, &b| (sk.head[a] - x).len().total_cmp(&(sk.head[b] - x).len()).then(a.cmp(&b)));
            if let Some(c) = child {
                let bw = radius[j].max(radius[c]).max(1e-6);
                let cdir = (sk.tail[c] - sk.head[c]).normalized();
                let s = (x - sk.head[c]).dot(cdir);
                if s > -bw {
                    let t = smoothstep((s + bw) / (2.0 * bw));
                    return normalize_vw(vec![(c as u32, t), (j as u32, 1.0 - t)]);
                }
            }
            vec![(j as u32, 1.0)]
        })
        .collect();
    let body: Vec<bool> = (0..model.nverts()).map(|v| !is_piece(model, v)).collect();
    let w = prune_all(&smooth(model, &w, &body, 0, 0.5));
    prune_all(&pieces_from_body(model, ctx, &w))
}

/// Skin weighted to a bone it does not belong to (the check's wrong-bone
/// rule: a poncho panel on the forearm, a thigh on the other thigh) keeps
/// only its own bones' weights, then two smoothing passes over the changed
/// patch. The region choice keeps it where it tears less.
pub fn own_bones(model: &Model, ctx: &Ctx) -> Weights {
    let (_, _, foreign) = crate::owner::own_bones(model, ctx, &model.weights);
    let mut w = model.weights.clone();
    let mut changed = vec![false; model.nverts()];
    for (v, f) in foreign.into_iter().enumerate() {
        if let Some((home, _)) = f {
            w[v] = home;
            changed[v] = true;
        }
    }
    if !changed.iter().any(|&c| c) {
        return w;
    }
    let patch = dilate(model, &changed, 1);
    prune_all(&smooth(model, &w, &patch, 2, 0.5))
}

/// The fused crotch (every generated cast rig: inner-thigh skin moving with
/// the other thigh, 7-47 cm): on a leg's skin, weight on the other leg
/// (its thigh and everything under it) goes to the pelvis instead, then two
/// smoothing passes over the changed patch. The skin bridging the legs
/// rides the pelvis rather than being torn between two thighs. Humanoids
/// with both thighs and a hips joint only.
pub fn crotch_to_pelvis(model: &Model) -> Option<Weights> {
    let sk = &model.skel;
    let a = &sk.anatomy;
    let hips = *a.roles.get("hips")?.first()? as u32;
    let thigh = [*a.roles.get("thigh.L")?.first()? as u32, *a.roles.get("thigh.R")?.first()? as u32];
    let leg_of = |j: usize| -> Option<usize> {
        match &a.group[crate::helpers::owner(sk, j)] {
            crate::anatomy::Group::Limb(n) if n == "leg.L" => Some(0),
            crate::anatomy::Group::Limb(n) if n == "leg.R" => Some(1),
            _ => None,
        }
    };
    let left = a.up.cross(a.forward).normalized();
    let mid = sk.head[hips as usize].dot(left);
    // Half the gap between the hip joints: the band a fused crotch spans.
    let half = 0.5 * (sk.head[thigh[0] as usize] - sk.head[thigh[1] as usize]).dot(left).abs();
    if half <= 0.0 {
        return None;
    }
    let top_y = sk.head[thigh[0] as usize].y.max(sk.head[thigh[1] as usize].y);
    let mut changed = vec![false; model.nverts()];
    let mut w: Weights = model.weights.clone();
    for v in 0..model.nverts() {
        if model.rigid[v] || model.rest[v].y > top_y {
            continue;
        }
        if !w[v].iter().any(|&(j, _)| leg_of(j as usize).is_some()) {
            continue;
        }
        // The leg it belongs to: the side of the body it is on.
        let lat = model.rest[v].dot(left) - mid;
        let own = if lat >= 0.0 { 0 } else { 1 };
        let mut other = 0.0;
        let mut keep: VW = Vec::new();
        for &(j, x) in &w[v] {
            if leg_of(j as usize).is_some_and(|l| l != own) {
                other += x;
            } else {
                keep.push((j, x));
            }
        }
        if other <= 0.0 {
            continue;
        }
        // Inside the band a smooth share goes to the pelvis and the other
        // thigh keeps the rest, fading to nothing at the band's edge; beyond
        // it, all to the pelvis.
        let t = (lat.abs() / half).clamp(0.0, 1.0);
        let stay = 0.5 * (1.0 - t * t * (3.0 - 2.0 * t));
        keep.push((thigh[1 - own], other * stay));
        keep.push((hips, other * (1.0 - stay)));
        w[v] = normalize_vw(keep);
        changed[v] = true;
    }
    if !changed.iter().any(|&c| c) {
        return None;
    }
    let patch = dilate(model, &changed, 1);
    Some(prune_all(&smooth(model, &w, &patch, 2, 0.5)))
}

/// One side's weights mirrored onto the other (`from_left`: the left
/// side's onto the right), partner bones swapped: `.L` roles for `.R`
/// roles, other bones by their mirrored head. A vertex takes the weights
/// of the vertex nearest its reflection; the midline (within 1% of the
/// height) keeps its own. Humanoids only (None otherwise). Averaging the
/// two sides was tried 2026-11-11 and blurs two boundaries into a band
/// that collapses; copying one keeps a boundary whole.
pub fn mirror(model: &Model, from_left: bool) -> Option<Weights> {
    let sk = &model.skel;
    let a = &sk.anatomy;
    if a.body != crate::anatomy::Body::Humanoid {
        return None;
    }
    let left = a.up.cross(a.forward).normalized();
    let c = sk.head[*a.roles.get("hips")?.first()?];
    let refl = |p: Vec3| p - left * (2.0 * (p - c).dot(left));
    let n = model.njoints();
    let mut partner: Vec<u32> = (0..n as u32).collect();
    for (r, js) in &a.roles {
        if let Some(base) = r.strip_suffix(".L")
            && let Some(other) = a.roles.get(&format!("{base}.R"))
        {
            for (i, &j) in js.iter().enumerate() {
                if let Some(&o) = other.get(i) {
                    // A rig posed asymmetrically at rest (a striding
                    // figure) cannot copy weights across its midline.
                    if ["thigh", "shin", "upper_arm", "forearm"].contains(&base)
                        && ((sk.head[o] - refl(sk.head[j])).len() > 0.06 * model.scale
                            || (sk.tail[o] - refl(sk.tail[j])).len() > 0.08 * model.scale)
                    {
                        return None;
                    }
                    partner[j] = o as u32;
                    partner[o] = j as u32;
                }
            }
        }
    }
    let in_role: std::collections::BTreeSet<usize> = a.roles.values().flatten().copied().collect();
    for (j, slot) in partner.iter_mut().enumerate() {
        if in_role.contains(&j) {
            continue;
        }
        let h = refl(sk.head[j]);
        if let Some((k, d)) = (0..n).map(|k| (k, (sk.head[k] - h).len())).min_by(|x, y| x.1.total_cmp(&y.1).then(x.0.cmp(&y.0)))
            && d < 0.03 * model.scale
        {
            *slot = k as u32;
        }
    }
    let h = crate::owner::height(model);
    let cell = model.scale * 0.01;
    let key = |p: Vec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
    let mut grid: BTreeMap<(i64, i64, i64), Vec<usize>> = BTreeMap::new();
    for (i, p) in model.rest.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i);
    }
    let nearest = |p: Vec3| -> Option<usize> {
        let k = key(p);
        let mut best = (f64::MAX, usize::MAX);
        for r in 0..3i64 {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                            continue;
                        }
                        for &i in grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)).into_iter().flatten() {
                            let d = (model.rest[i] - p).len();
                            if d < best.0 || (d == best.0 && i < best.1) {
                                best = (d, i);
                            }
                        }
                    }
                }
            }
            if best.1 != usize::MAX {
                break;
            }
        }
        (best.1 != usize::MAX && best.0 < 0.02 * model.scale).then_some(best.1)
    };
    let mut asked = 0usize;
    let mut matched = 0usize;
    let w: Weights = (0..model.nverts())
        .map(|v| {
            let lat = (model.rest[v] - c).dot(left);
            let receives = if from_left { lat < -0.01 * h } else { lat > 0.01 * h };
            if !receives || model.rigid[v] {
                return model.weights[v].clone();
            }
            asked += 1;
            match nearest(refl(model.rest[v])) {
                Some(u) => {
                    matched += 1;
                    normalize_vw(model.weights[u].iter().map(|&(j, x)| (partner[j as usize], x)).collect())
                }
                None => model.weights[v].clone(),
            }
        })
        .collect();
    (asked > 0 && matched * 20 >= asked * 17).then_some(w)
}

/// Maps source joints onto target joints: by name; else by structural
/// role (`anatomy.rs`: a Mixamo `LeftUpLeg` and a `DEF-thigh.L` are both
/// `thigh.L`; chains by their place along the chain); else by nearest rest
/// head within 5% of the target's size; else onto whatever the joint's
/// nearest mapped ancestor maps to (a finger the target lacks goes to its
/// hand), so no weight is dropped.
pub fn map_joints(src: &Model, dst: &Model) -> Vec<Option<u32>> {
    let roles_of = |m: &Model| -> std::collections::BTreeMap<usize, (String, usize, usize)> {
        let mut out = std::collections::BTreeMap::new();
        for (r, js) in &m.skel.anatomy.roles {
            for (i, &j) in js.iter().enumerate() {
                out.entry(j).or_insert((r.clone(), i, js.len()));
            }
        }
        out
    };
    let (rs, rd) = (roles_of(src), &dst.skel.anatomy.roles);
    let mut map: Vec<Option<u32>> = (0..src.njoints())
        .map(|j| {
            if let Some(t) = dst.skel.joint_by_name(&src.skel.names[j]) {
                return Some(t as u32);
            }
            if let Some((r, i, n)) = rs.get(&j)
                && let Some(dj) = rd.get(r).filter(|v| !v.is_empty())
            {
                let k = if *n <= 1 { 0 } else { ((*i as f64 / (*n - 1) as f64) * (dj.len() - 1) as f64).round() as usize };
                return Some(dj[k.min(dj.len() - 1)] as u32);
            }
            let h = src.skel.head[j];
            let (best, d) =
                (0..dst.njoints()).map(|t| (t, (dst.skel.head[t] - h).len())).min_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)))?;
            (d <= 0.05 * dst.scale).then_some(best as u32)
        })
        .collect();
    for j in 0..src.njoints() {
        if map[j].is_none() {
            let mut p = src.skel.jparent[j];
            while let Some(q) = p {
                if let Some(t) = map[q] {
                    map[j] = Some(t);
                    break;
                }
                p = src.skel.jparent[q];
            }
        }
    }
    map
}

/// Method 3, transfer + inpaint (Abdrashitov et al. 2023). With a source
/// rig: body vertices take the source's weights where the surfaces match
/// (close, same facing) and are inpainted elsewhere. Pieces always get
/// robust transfer from the (new) body.
pub fn transfer(model: &Model, ctx: &Ctx, source: Option<&Model>, match_dist: f64) -> Weights {
    let mut w = model.weights.clone();
    if let Some(src) = source {
        let jm = map_joints(src, model);
        let src_w: Weights =
            src.weights.iter().map(|vw| normalize_vw(vw.iter().filter_map(|&(j, x)| jm[j as usize].map(|t| (t, x))).collect())).collect();
        let body: Vec<u32> = (0..model.nverts() as u32).filter(|&v| !is_piece(model, v as usize)).collect();
        let src_tris: Vec<[u32; 3]> = src.tris.iter().filter(|t| t.iter().all(|&i| !is_piece(src, i as usize))).copied().collect();
        let sn = crate::scene::vertex_normals(&src.rest, &src.tris);
        let tn = crate::scene::vertex_normals(&model.rest, &model.tris);
        let index = TriIndex::new(&src.rest, src_tris);
        let matches = match_subset(&index, &sn, &model.rest, &tn, &body, match_dist * model.scale, 35f64.to_radians().cos());
        let r = inpaint_from_matches(&model.adj, &matches, &src_w);
        for m in &matches {
            w[m.0 as usize] = r[m.0 as usize].clone();
        }
    }
    prune_all(&pieces_from_body(model, ctx, &w))
}

/// Euclidean projection of v onto the probability simplex.
fn project_simplex(v: &mut [f64]) {
    let mut u: Vec<f64> = v.to_vec();
    u.sort_by(|a, b| b.total_cmp(a));
    let mut css = 0.0;
    let mut theta = 0.0;
    for (i, &ui) in u.iter().enumerate() {
        css += ui;
        let t = (css - 1.0) / (i + 1) as f64;
        if ui - t > 0.0 {
            theta = t;
        }
    }
    for x in v.iter_mut() {
        *x = (*x - theta).max(0.0);
    }
}

/// Method 4, optimize: per vertex, fit linear-blend weights to the dual
/// quaternion deformation of the start weights over the whole pose set
/// (DQS does not collapse at bends or twists), with smoothness and
/// stay-close terms, weights on the simplex, bones limited to the start
/// set plus geodesically near ones. Only flagged areas move.
pub fn optimize(model: &Model, ctx: &Ctx, start: &Weights, ev: &Eval) -> Weights {
    let flagged: Vec<bool> = ev.flags.iter().map(|&f| f & !crate::metrics::F_INTERSECT != 0).collect();
    let active = grow(model, &flagged, widths::OPTIMIZE_MARGIN * ctx.feature, None);
    let targets: Vec<Vec<Vec3>> = ctx.mats.par_iter().map(|m| dqs(model, m, start)).collect();
    let np = ctx.poses.len().max(1) as f64;
    let s2 = model.scale * model.scale;
    let (ls, l0) = (0.05, 0.01);
    let mut cur = start.clone();
    for _sweep in 0..4 {
        let prev = cur.clone();
        cur = (0..model.nverts())
            .into_par_iter()
            .map(|v| {
                if !active[v] || is_piece(model, v) {
                    return prev[v].clone();
                }
                // Candidate bones.
                let mut bones: Vec<u32> = start[v].iter().map(|e| e.0).collect();
                if let Some(near) = ctx.geo.nearest(v) {
                    for &(j, d) in ctx.geo.near[v].iter().take(4) {
                        if ((d - near.1) as f64) < ctx.th.bleed_excess * model.scale * 0.5 && !bones.contains(&j) {
                            bones.push(j);
                        }
                    }
                }
                bones.truncate(6);
                let k = bones.len();
                if k <= 1 {
                    return prev[v].clone();
                }
                let x = model.rest[v];
                // H = sum_p A_p^T A_p / (P s^2) + (ls + l0) I ; g likewise.
                let mut h = vec![0.0; k * k];
                let mut g = vec![0.0; k];
                for (p, m) in ctx.mats.iter().enumerate() {
                    let cols: Vec<Vec3> = bones.iter().map(|&j| m[j as usize].transform_point(x)).collect();
                    let t = targets[p][v];
                    for a in 0..k {
                        g[a] += cols[a].dot(t) / (np * s2);
                        for b in 0..k {
                            h[a * k + b] += cols[a].dot(cols[b]) / (np * s2);
                        }
                    }
                }
                let n = &model.adj[v];
                let kn = n.len().max(1) as f64;
                for (a, &j) in bones.iter().enumerate() {
                    let mean = n.iter().map(|&u| weight_of(&prev[u as usize], j)).sum::<f64>() / kn;
                    g[a] += ls * mean + l0 * weight_of(&start[v], j);
                    h[a * k + a] += ls + l0;
                }
                let lip: f64 = (0..k).map(|a| h[a * k + a]).sum::<f64>().max(1e-12);
                let mut wv: Vec<f64> = bones.iter().map(|&j| weight_of(&prev[v], j)).collect();
                project_simplex(&mut wv);
                for _ in 0..200 {
                    let grad: Vec<f64> = (0..k).map(|a| (0..k).map(|b| h[a * k + b] * wv[b]).sum::<f64>() - g[a]).collect();
                    for a in 0..k {
                        wv[a] -= grad[a] / lip;
                    }
                    project_simplex(&mut wv);
                }
                let mut out: VW = bones.iter().zip(&wv).map(|(&j, &x)| (j, x)).collect();
                sort_vw(&mut out);
                prune_vw(&out, 4, 0.01)
            })
            .collect();
    }
    prune_all(&cur)
}

/// External candidate (e.g. SkinTokens output) on a possibly different mesh
/// and skeleton: joints mapped by name or head position, weights moved by
/// robust transfer.
pub fn external(model: &Model, ctx: &Ctx, other: &Model, match_dist: f64) -> Weights {
    transfer(model, ctx, Some(other), match_dist)
}
