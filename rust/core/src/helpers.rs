//! Twist/helper bones (the standard humanoid skeleton's `DEF-upper_arm_twist.L`,
//! `DEF-thigh_twist.R`, ...).
//!
//! A helper sits on its driver's joint, under the driver's parent, and
//! turns by a share of the driver's rotation (motionforge
//! `core/src/helpers.rs` inserts them and bakes that rotation into every
//! clip). weightforge has to pose them the same way, or its ROM would
//! leave the helpers at rest and measure a rig the game never shows.
//!
//! Detection: node `extras.helper_bone = {"driver", "share"}`, else the
//! name convention `<driver base>_twist.<side>` with share 0.5. A helper
//! must hang from its driver's parent; anything else is a plain bone.
//!
//! Weights: helpers are not a region of their own (their vertices belong
//! to the driver's region), not a pose role, and every generated fix
//! candidate is re-blended onto them with the same rule motionforge uses
//! (`reblend`), so a candidate never drops the helpers.

use crate::math::{Mat4, Quat};
use crate::metrics::Ctx;
use crate::scene::{Model, Node, Skeleton, VW, Weights, normalize_vw, prune_vw};
use serde_json::Value;

pub const DEFAULT_SHARE: f64 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub struct Helper {
    /// Joint indices (skin order).
    pub joint: usize,
    pub driver: usize,
    pub share: f64,
}

/// Helpers among the skin's joints.
pub fn detect(raw_nodes: &[Value], nodes: &[Node], joints: &[usize], names: &[String]) -> Vec<Helper> {
    let mut out = Vec::new();
    for (j, &n) in joints.iter().enumerate() {
        let extras = raw_nodes.get(n).and_then(|x| x.get("extras")).and_then(|e| e.get("helper_bone"));
        let (driver_name, share) = match extras {
            Some(h) => (
                h.get("driver").and_then(Value::as_str).map(str::to_string),
                h.get("share").and_then(Value::as_f64).unwrap_or(DEFAULT_SHARE),
            ),
            None => (by_name(&names[j]), DEFAULT_SHARE),
        };
        let Some(driver_name) = driver_name else { continue };
        let Some(d) = names.iter().position(|x| *x == driver_name) else { continue };
        if d == j || nodes[joints[d]].parent != nodes[n].parent || !(0.0..=1.0).contains(&share) {
            continue;
        }
        out.push(Helper { joint: j, driver: d, share });
    }
    out
}

/// `DEF-upper_arm_twist.L` -> `DEF-upper_arm.L`.
fn by_name(name: &str) -> Option<String> {
    let (base, side) = name.rsplit_once('.')?;
    if side != "L" && side != "R" {
        return None;
    }
    let driver = base.strip_suffix("_twist")?;
    Some(format!("{driver}.{side}"))
}

pub fn helper_of(sk: &Skeleton, j: usize) -> Option<&Helper> {
    sk.helpers.iter().find(|h| h.joint == j)
}

/// The joint a vertex region or pose role should use for `j`.
pub fn owner(sk: &Skeleton, j: usize) -> usize {
    helper_of(sk, j).map(|h| h.driver).unwrap_or(j)
}

/// Pose the helpers in node globals `g` from their drivers. `keyed[node]`
/// marks nodes the pose sets explicitly (a baked clip), which win.
pub fn drive(sk: &Skeleton, g: &mut [Mat4], keyed: &[bool]) {
    for h in &sk.helpers {
        let (hn, dn) = (sk.joints[h.joint], sk.joints[h.driver]);
        if keyed.get(hn).copied().unwrap_or(false) {
            continue;
        }
        let parent = match sk.nodes[dn].parent {
            Some(p) => g[p],
            None => Mat4::IDENTITY,
        };
        let posed = parent.inverse() * g[dn];
        let rest = sk.nodes[dn].local();
        let delta = posed.rotation() * rest.rotation().conj();
        let part = Quat::IDENTITY.slerp(delta.normalized(), h.share).normalized();
        let hr = &sk.nodes[hn];
        let t = hr.t + (posed.translation() - rest.translation());
        g[hn] = parent * Mat4::from_trs(t, (part * hr.r).normalized(), hr.s);
    }
}

/// Re-blend one vertex over a helper (motionforge `helpers::reblend`).
/// Returns (ancestor scale, helper weight, driver weight).
pub fn reblend(limb: f64, prox: f64, share: f64) -> (f64, f64, f64) {
    let total = limb + prox;
    if limb <= 0.0 || prox <= 0.0 || total <= 0.0 {
        return (1.0, 0.0, limb);
    }
    let a = limb / total;
    if a <= share {
        let h = total * a / share;
        ((total - h) / prox, h, 0.0)
    } else {
        let h = total * (1.0 - a) / (1.0 - share);
        (0.0, h, total - h)
    }
}

fn is_ancestor(sk: &Skeleton, a: usize, mut j: usize) -> bool {
    let mut guard = 0;
    while let Some(p) = sk.jparent[j] {
        if p == a {
            return true;
        }
        j = p;
        guard += 1;
        if guard > sk.jparent.len() {
            break;
        }
    }
    false
}

/// Per helper: joints outside its driver's subtree (and not a helper).
/// In a pose that turns only the driver these all stay put, so their
/// weight is the "body" side of the joint (chest, hips, the other thigh).
fn outside(sk: &Skeleton) -> Vec<Vec<bool>> {
    sk.helpers
        .iter()
        .map(|h| (0..sk.joints.len()).map(|j| j != h.driver && !is_ancestor(sk, h.driver, j) && helper_of(sk, j).is_none()).collect())
        .collect()
}

/// Narrow armpit/deltoid repair: shifts lower-torso spine bleed (`spine`,
/// `spine.001`, `spine.002`) on the armpit seam into `shoulder` + `chest`,
/// smooths a tight (`< 1` limb radius) shoulder-to-arm transition so overhead
/// arms never stretch past 3.5x or trigger >1.5-radius volume collapse, and
/// re-blends arm twist helpers. Legs and torso below the armpit are untouched.
pub fn arm_shoulder_band(model: &Model, ctx: &Ctx, w: &Weights, width: f64) -> Weights {
    let sk = &model.skel;
    let chest = sk.anatomy.roles.get("chest").and_then(|v| v.first().copied());
    let mut lower_torso = vec![false; sk.joints.len()];
    for role in ["hips", "spine*"] {
        if let Some(js) = sk.anatomy.roles.get(role) {
            for &j in js {
                if Some(j) != chest {
                    lower_torso[j] = true;
                }
            }
        }
    }
    // Fold helpers into drivers on arms first (keep leg weights intact).
    let mut cur: Weights = w
        .iter()
        .map(|vw| {
            let mut acc: VW = Vec::new();
            for &(j, x) in vw {
                let jo = if !sk.names[owner(sk, j as usize)].contains("thigh") { owner(sk, j as usize) as u32 } else { j };
                match acc.iter_mut().find(|e| e.0 == jo) {
                    Some(e) => e.1 += x,
                    None => acc.push((jo, x)),
                }
            }
            acc
        })
        .collect();
    let nv = model.nverts();
    let smooth01 = |t: f64| {
        let x = t.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let mut hips_or_head = vec![false; sk.joints.len()];
    for role in ["hips", "head", "neck"] {
        if let Some(js) = sk.anatomy.roles.get(role) {
            for &j in js {
                hips_or_head[j] = true;
            }
        }
    }
    let arm_drivers: Vec<usize> =
        sk.anatomy.roles.iter().filter(|(r, _)| r.starts_with("upper_arm.")).flat_map(|(_, js)| js.iter().copied()).collect();
    for d_idx in arm_drivers {
        let d = d_idx as u32;
        let parent = sk.jparent[d_idx].unwrap_or(d_idx) as u32;
        let arm_chest = chest.unwrap_or(parent as usize) as u32;
        let radius = ctx
            .regions
            .iter()
            .position(|r| r.name == sk.names[d_idx])
            .map(|r| ctx.region_radius[r])
            .filter(|&x| x > 0.0)
            .unwrap_or(ctx.feature);
        let arm_head = sk.head[d_idx];
        let up = sk.anatomy.up;
        let body_set: Vec<bool> =
            (0..sk.joints.len()).map(|j| j != d_idx && !is_ancestor(sk, d_idx, j) && helper_of(sk, j).is_none()).collect();
        let limb: Vec<f64> = cur.iter().map(|vw| vw.iter().filter(|e| e.0 == d).map(|e| e.1).sum()).collect();
        let body: Vec<f64> = cur.iter().map(|vw| vw.iter().filter(|e| body_set[e.0 as usize]).map(|e| e.1).sum()).collect();
        let low: Vec<f64> = cur.iter().map(|vw| vw.iter().filter(|e| lower_torso[e.0 as usize]).map(|e| e.1).sum()).collect();
        let keep_out: Vec<f64> = cur.iter().map(|vw| vw.iter().filter(|e| hips_or_head[e.0 as usize]).map(|e| e.1).sum()).collect();
        // Only vertices touching the armpit/deltoid seam; grow outward along both
        // the arm and upper ribcage so `!zone` pins `a0` (~1.0 on arm, ~0.0 on torso).
        let joint: Vec<bool> = (0..nv)
            .map(|v| limb[v] + body[v] >= 0.5 && keep_out[v] < 0.25 && (model.rest[v] - arm_head).dot(up) >= -2.2 * radius)
            .collect();
        let mixed: Vec<bool> = (0..nv)
            .map(|v| {
                joint[v]
                    && ((limb[v] > 0.02 && limb[v] < 0.85 && (body[v] > 0.02 || low[v] > 0.02))
                        || model.adj[v].iter().any(|&u| joint[u as usize] && ((limb[v] > 0.5) != (limb[u as usize] > 0.5))))
            })
            .collect();
        let zone = crate::fix::grow(model, &mixed, width * 1.15 * radius, Some(&joint));
        let mut tail = crate::fix::dilate(model, &zone, 2);
        for v in 0..nv {
            if keep_out[v] >= 0.25 || limb[v] + body[v] < 0.5 {
                tail[v] = false;
            }
        }
        // Harmonic Laplace interpolation inside `zone`, then C^1 feathering across `tail`
        // so `a` meets untouched arm and torso boundaries with zero slope discontinuity.
        let a0: Vec<f64> = (0..nv)
            .map(|v| {
                let t = limb[v] + body[v];
                if t > 1e-9 { limb[v] / t } else { 0.0 }
            })
            .collect();
        let mut a: Vec<f64> = a0.clone();
        for _ in 0..64 {
            let prev = a.clone();
            for v in 0..nv {
                if !zone[v] {
                    continue;
                }
                let ns = &model.adj[v];
                if ns.is_empty() {
                    continue;
                }
                a[v] = ns.iter().map(|&u| prev[u as usize]).sum::<f64>() / ns.len() as f64;
            }
        }
        for _ in 0..10 {
            let prev = a.clone();
            for v in 0..nv {
                if !tail[v] {
                    continue;
                }
                let ns = &model.adj[v];
                if ns.is_empty() {
                    continue;
                }
                a[v] = ns.iter().map(|&u| prev[u as usize]).sum::<f64>() / ns.len() as f64;
            }
        }
        for v in 0..nv {
            if !tail[v] {
                continue;
            }
            let total = limb[v] + body[v];
            let (nl, nb) = (a[v] * total, (1.0 - a[v]) * total);
            let vw = &mut cur[v];
            let keep_low = smooth01((0.55 - a[v]) / 0.40);
            let low_sum: f64 = vw.iter().filter(|e| lower_torso[e.0 as usize]).map(|e| e.1).sum();
            let target_low = if body[v] > 1e-9 { (low_sum / body[v]) * nb * keep_low } else { 0.0 };
            let dropped_low = if body[v] > 1e-9 { (low_sum / body[v]) * nb * (1.0 - keep_low) } else { 0.0 };
            let rest_body = (nb - target_low).max(0.0);
            if low_sum > 1e-9 {
                let fl = target_low / low_sum;
                for e in vw.iter_mut() {
                    if lower_torso[e.0 as usize] {
                        e.1 *= fl;
                    }
                }
            }
            let other_body: f64 = vw.iter().filter(|e| body_set[e.0 as usize] && !lower_torso[e.0 as usize]).map(|e| e.1).sum();
            let keep_other = (rest_body - dropped_low).max(0.0);
            if other_body > 1e-9 {
                let f = keep_other / other_body;
                for e in vw.iter_mut() {
                    if body_set[e.0 as usize] && !lower_torso[e.0 as usize] {
                        e.1 *= f;
                    }
                }
            }
            let add_girdle = dropped_low + if other_body <= 1e-9 { keep_other } else { 0.0 };
            if add_girdle > 0.0 {
                if parent != arm_chest {
                    let sh_w = add_girdle * 0.72;
                    let ch_w = add_girdle - sh_w;
                    vw.push((parent, sh_w));
                    vw.push((arm_chest, ch_w));
                } else {
                    vw.push((parent, add_girdle));
                }
            }
            match vw.iter_mut().find(|e| e.0 == d) {
                Some(e) => e.1 = nl,
                None => vw.push((d, nl)),
            }
            let mut m: VW = Vec::new();
            for &(j, x) in vw.iter() {
                match m.iter_mut().find(|e| e.0 == j) {
                    Some(e) => e.1 += x,
                    None => m.push((j, x)),
                }
            }
            *vw = m;
        }
    }
    let rb = reassign(sk, &cur);
    let mut out = w.clone();
    for v in 0..nv {
        if cur[v] != w[v] {
            out[v] = rb[v].clone();
        }
    }
    out
}

/// Band widths `(zone, smoothing)` in limb radii of the driver (the median
/// distance of its region's vertices to its bone), so a band keeps its
/// width at any mesh density. Measured on the SkinTokens hero game mesh
/// (README "Twist/helper bones"), where they were 6 rings / 20 passes
/// (arms) and 4 rings / 30 passes (legs): wider arm bands trade stretch
/// for collapse, wider leg bands drag the shirt hem.
pub const ARM_BAND: (f64, f64) = (2.55, 0.95);
pub const LEG_BAND: (f64, f64) = (1.6, 1.1);

/// Fix candidate: widen each driver's joint band (`width` scales both
/// band widths; `fix` tries a few), then re-blend it onto the helper. Around the joint, the driver's share `a = limb / (limb +
/// body)` is smoothed over the mesh (within a zone around the vertices that
/// mix the two or sit on a hard edge between them, then Laplacian passes
/// for the band's width), so the armpit and groin turn gradually instead
/// of tearing; the helper then keeps that wider blend from collapsing
/// (`reassign`). Identity without helpers.
pub fn band(model: &Model, ctx: &Ctx, w: &Weights, width: f64) -> Weights {
    let sk = &model.skel;
    if sk.helpers.is_empty() {
        return w.clone();
    }
    let out = outside(sk);
    // Fold helpers into their drivers first.
    let mut cur: Weights = w
        .iter()
        .map(|vw| {
            let mut acc: VW = Vec::new();
            for &(j, x) in vw {
                let j = owner(sk, j as usize) as u32;
                match acc.iter_mut().find(|e| e.0 == j) {
                    Some(e) => e.1 += x,
                    None => acc.push((j, x)),
                }
            }
            acc
        })
        .collect();
    let nv = model.nverts();
    let chest = sk.anatomy.roles.get("chest").and_then(|v| v.first().copied());
    let mut lower_torso = vec![false; sk.joints.len()];
    for role in ["hips", "spine*"] {
        if let Some(js) = sk.anatomy.roles.get(role) {
            for &j in js {
                if Some(j) != chest {
                    lower_torso[j] = true;
                }
            }
        }
    }
    for (k, h) in sk.helpers.iter().enumerate() {
        let d = h.driver as u32;
        // Legs: the body side is the hips chain only, so the band never
        // spreads one thigh into the other across the crotch.
        let is_leg = sk.names[h.driver].contains("thigh");
        let (zone_r, smooth_r) = if is_leg { LEG_BAND } else { (1.05, 0.45) };
        let radius = ctx
            .regions
            .iter()
            .position(|r| r.name == sk.names[h.driver])
            .map(|r| ctx.region_radius[r])
            .filter(|&x| x > 0.0)
            .unwrap_or(ctx.feature);
        let body_set: Vec<bool> =
            if is_leg { (0..sk.joints.len()).map(|j| j != h.driver && is_ancestor(sk, j, h.driver)).collect() } else { out[k].clone() };
        let parent = sk.jparent[h.driver].unwrap_or(h.driver) as u32;
        let arm_chest = if !is_leg { chest.unwrap_or(parent as usize) as u32 } else { parent };
        let limb: Vec<f64> = cur.iter().map(|vw| vw.iter().filter(|e| e.0 == d).map(|e| e.1).sum()).collect();
        let body: Vec<f64> = cur.iter().map(|vw| vw.iter().filter(|e| body_set[e.0 as usize]).map(|e| e.1).sum()).collect();
        let low: Vec<f64> = if is_leg {
            vec![0.0; nv]
        } else {
            cur.iter().map(|vw| vw.iter().filter(|e| lower_torso[e.0 as usize]).map(|e| e.1).sum()).collect()
        };
        let girdle: Vec<f64> = if is_leg {
            vec![0.0; nv]
        } else {
            cur.iter().map(|vw| vw.iter().filter(|e| e.0 == parent || e.0 == arm_chest).map(|e| e.1).sum()).collect()
        };
        let joint: Vec<bool> = (0..nv)
            .map(|v| {
                if limb[v] + body[v] < 0.5 {
                    return false;
                }
                if is_leg {
                    return true;
                }
                // Keep arm bands strictly on the shoulder/chest/armpit side so
                // upper_arm never bleeds across empty air onto the lower spine.
                limb[v] > 0.04 || girdle[v] > 0.20
            })
            .collect();
        let share_of = |v: usize| limb[v] / (limb[v] + body[v]).max(1e-12);
        // Seeds: vertices that mix the two, and both ends of a hard edge
        // (all-limb next to all-body).
        let mixed: Vec<bool> = (0..nv)
            .map(|v| {
                joint[v]
                    && ((limb[v] > 0.02 && body[v] > 0.02)
                        || model.adj[v].iter().any(|&u| joint[u as usize] && (share_of(v) - share_of(u as usize)).abs() > 0.5))
            })
            .collect();
        let zone = crate::fix::grow(model, &mixed, width * zone_r * radius, Some(&joint));
        let iters = crate::fix::passes(model, &zone, width * smooth_r * radius);
        if std::env::var_os("WF_DEBUG").is_some() {
            let (mut sum, mut cnt) = (0.0, 0);
            for e in &model.edges {
                if mixed[e[0] as usize] && mixed[e[1] as usize] {
                    sum += (model.rest[e[0] as usize] - model.rest[e[1] as usize]).len();
                    cnt += 1;
                }
            }
            eprintln!(
                "band {}: radius {radius:.4}, seed edge {:.4}, zone {} verts, {iters} passes",
                sk.names[h.driver],
                sum / cnt.max(1) as f64,
                zone.iter().filter(|&&z| z).count()
            );
        }
        let a0: Vec<f64> = (0..nv).map(|v| if joint[v] { limb[v] / (limb[v] + body[v]) } else { 0.0 }).collect();
        let mut a = a0.clone();
        for _ in 0..iters {
            let prev = a.clone();
            for v in 0..nv {
                if !zone[v] {
                    continue;
                }
                let ns: Vec<f64> = model.adj[v].iter().filter(|&&u| joint[u as usize]).map(|&u| prev[u as usize]).collect();
                if ns.is_empty() {
                    continue;
                }
                let mean = ns.iter().sum::<f64>() / ns.len() as f64;
                let mut next = 0.5 * prev[v] + 0.5 * mean;
                if !is_leg && low[v] > 0.50 && limb[v] < 0.08 {
                    next = next.min(a0[v]);
                }
                a[v] = next;
            }
        }
        let smooth01 = |t: f64| {
            let x = t.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        for v in 0..nv {
            if !zone[v] {
                continue;
            }
            let total = limb[v] + body[v];
            let (nl, nb) = (a[v] * total, (1.0 - a[v]) * total);
            let vw = &mut cur[v];
            if !is_leg {
                // On the arm/armpit side (a > 0.05), lower-torso spine bleed fades
                // smoothly into shoulder + upper chest instead of pinning the armpit,
                // while preserving existing shoulder vs chest balance where already present.
                let keep_low = smooth01((0.30 - a[v]) / 0.25);
                let low_sum: f64 = vw.iter().filter(|e| lower_torso[e.0 as usize]).map(|e| e.1).sum();
                let target_low = if body[v] > 1e-9 { (low_sum / body[v]) * nb * keep_low } else { 0.0 };
                let dropped_low = if body[v] > 1e-9 { (low_sum / body[v]) * nb * (1.0 - keep_low) } else { 0.0 };
                let rest_body = (nb - target_low).max(0.0);
                if low_sum > 1e-9 {
                    let fl = target_low / low_sum;
                    for e in vw.iter_mut() {
                        if lower_torso[e.0 as usize] {
                            e.1 *= fl;
                        }
                    }
                }
                let other_body: f64 = vw.iter().filter(|e| body_set[e.0 as usize] && !lower_torso[e.0 as usize]).map(|e| e.1).sum();
                let keep_other = (rest_body - dropped_low).max(0.0);
                if other_body > 1e-9 {
                    let f = keep_other / other_body;
                    for e in vw.iter_mut() {
                        if body_set[e.0 as usize] && !lower_torso[e.0 as usize] {
                            e.1 *= f;
                        }
                    }
                }
                let add_girdle = dropped_low + if other_body <= 1e-9 { keep_other } else { 0.0 };
                if add_girdle > 0.0 {
                    if parent != arm_chest {
                        let sh_w = add_girdle * 0.70;
                        let ch_w = add_girdle - sh_w;
                        vw.push((parent, sh_w));
                        vw.push((arm_chest, ch_w));
                    } else {
                        vw.push((parent, add_girdle));
                    }
                }
            } else if body[v] > 1e-9 {
                let f = nb / body[v];
                for e in vw.iter_mut() {
                    if body_set[e.0 as usize] {
                        e.1 *= f;
                    }
                }
            } else if nb > 0.0 {
                vw.push((parent, nb));
            }
            match vw.iter_mut().find(|e| e.0 == d) {
                Some(e) => e.1 = nl,
                None => vw.push((d, nl)),
            }
            // Merge duplicates (the parent may already be listed).
            let mut m: VW = Vec::new();
            for &(j, x) in vw.iter() {
                match m.iter_mut().find(|e| e.0 == j) {
                    Some(e) => e.1 += x,
                    None => m.push((j, x)),
                }
            }
            *vw = m;
        }
    }
    reassign(sk, &cur)
}

/// Joints that are the body for every helper: outside all helper
/// drivers' subtrees (so never another limb) and not a helper.
fn body_joints(sk: &Skeleton) -> Vec<bool> {
    (0..sk.joints.len())
        .map(|j| helper_of(sk, j).is_none() && !sk.helpers.iter().any(|h| j == h.driver || is_ancestor(sk, h.driver, j)))
        .collect()
}

/// Fold helper weight back into the drivers, then re-blend every vertex
/// that mixes a driver with the body. Identity without helpers.
///
/// All helpers of a vertex are re-blended at once: the body weight is
/// shared among the drivers on the vertex in proportion to their weight,
/// and each driver re-blends against its share. So a crotch vertex on both
/// thighs and the hips comes out the same on both sides whatever order the
/// helpers are listed in, and as motion (`equivalent`) it keeps its input
/// when the body weight sits on the helpers' parent (the hips). The body
/// is never another limb: run one after another, the first thigh's
/// helper counted the other thigh as body and drained it (hips 0.4,
/// thighs 0.3 / 0.3 came out as motion 0.3 / 0.171).
pub fn reassign(sk: &Skeleton, w: &Weights) -> Weights {
    if sk.helpers.is_empty() {
        return w.clone();
    }
    let body = body_joints(sk);
    w.iter()
        .map(|vw| {
            let mut acc: VW = Vec::new();
            for &(j, x) in vw {
                let j = owner(sk, j as usize) as u32;
                match acc.iter_mut().find(|e| e.0 == j) {
                    Some(e) => e.1 += x,
                    None => acc.push((j, x)),
                }
            }
            let on = |acc: &VW, j: usize| -> f64 { acc.iter().filter(|e| e.0 as usize == j).map(|e| e.1).sum() };
            let limbs: Vec<f64> = sk.helpers.iter().map(|h| on(&acc, h.driver)).collect();
            let body_total: f64 = acc.iter().filter(|e| body[e.0 as usize]).map(|e| e.1).sum();
            let limb_total: f64 = limbs.iter().filter(|&&l| l > 0.0).sum();
            if body_total > 0.0 && limb_total > 0.0 {
                let mut kept = 0.0;
                let mut added: VW = Vec::new();
                for (k, h) in sk.helpers.iter().enumerate() {
                    if limbs[k] <= 0.0 {
                        continue;
                    }
                    let share_of_body = body_total * limbs[k] / limb_total;
                    let (scale, hw, dw) = reblend(limbs[k], share_of_body, h.share);
                    kept += scale * share_of_body;
                    for e in acc.iter_mut().filter(|e| e.0 as usize == h.driver) {
                        e.1 = dw;
                    }
                    added.push((h.joint as u32, hw));
                }
                let f = kept / body_total;
                for e in acc.iter_mut().filter(|e| body[e.0 as usize]) {
                    e.1 *= f;
                }
                acc.extend(added);
            }
            acc.retain(|e| e.1 > 0.0);
            normalize_vw(prune_vw(&acc, 4, 0.0))
        })
        .collect()
}

/// Weights as motion: each helper weight `x` becomes `share * x` on the
/// driver and the rest on the helper's parent joint (they turn the vertex
/// the same way). Identity without helpers.
pub fn equivalent<'a>(sk: &Skeleton, w: &'a Weights) -> std::borrow::Cow<'a, Weights> {
    if sk.helpers.is_empty() {
        return std::borrow::Cow::Borrowed(w);
    }
    let mut split: Vec<Option<(u32, u32, f64)>> = vec![None; sk.joints.len()];
    for h in &sk.helpers {
        let parent = sk.jparent[h.joint].unwrap_or(h.driver);
        split[h.joint] = Some((h.driver as u32, parent as u32, h.share));
    }
    std::borrow::Cow::Owned(
        w.iter()
            .map(|vw| {
                let mut acc: VW = Vec::with_capacity(vw.len() + 1);
                let mut add = |j: u32, x: f64| match acc.iter_mut().find(|e| e.0 == j) {
                    Some(e) => e.1 += x,
                    None => acc.push((j, x)),
                };
                for &(j, x) in vw {
                    match split[j as usize] {
                        Some((d, p, s)) => {
                            add(d, x * s);
                            add(p, x * (1.0 - s));
                        }
                        None => add(j, x),
                    }
                }
                acc
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{Bone, build, to_glb};
    use crate::metrics::{Ctx, CtxOpts, evaluate};

    /// The clean mannequin plus the standard arm and thigh helpers.
    fn with_helpers() -> Model {
        let mut f = build("clean");
        for side in ["L", "R"] {
            for (helper, driver) in [("DEF-upper_arm_twist", "DEF-upper_arm"), ("DEF-thigh_twist", "DEF-thigh")] {
                let name: &'static str = Box::leak(format!("{helper}.{side}").into_boxed_str());
                let d = f.bones.iter().position(|b| b.name == format!("{driver}.{side}")).unwrap();
                let b = &f.bones[d];
                f.bones.push(Bone { name, parent: b.parent, head: b.head, tail: b.tail });
            }
        }
        Model::from_glb(to_glb(&f)).unwrap()
    }

    #[test]
    fn helpers_are_found_and_turn_half_their_driver() {
        let m = with_helpers();
        assert_eq!(m.skel.helpers.len(), 4);
        let set = crate::poses::parse_pose_set(
            "PoseSet(name: \"t\", poses: [PoseSpec(name: \"up\", moves: [Move(bone: \"upper_arm.L\", motion: Bend(Up, 80))])])",
        )
        .unwrap();
        let poses = crate::poses::resolve(&set, &m.skel);
        // The helper is not a pose role: the move lands on the real upper arm.
        assert_eq!(m.skel.names[poses[0].moves[0].0], "DEF-upper_arm.L");
        let mats = crate::skin::joint_matrices(&m, &poses[0]);
        let h = m.skel.helpers.iter().find(|h| m.skel.names[h.joint] == "DEF-upper_arm_twist.L").unwrap();
        let angle = |q: Quat| 2.0 * q.w.abs().min(1.0).acos().to_degrees();
        let (ad, ah) = (angle(mats[h.driver].rotation()), angle(mats[h.joint].rotation()));
        assert!((ad - 80.0).abs() < 1e-6, "driver {ad}");
        assert!((ah - 40.0).abs() < 1e-6, "helper {ah}");
        // Same pivot: the joint head does not move.
        let head = m.skel.head[h.joint];
        assert!((mats[h.joint].transform_point(head) - head).len() < 1e-9);
    }

    #[test]
    fn band_weights_helpers_and_regions_ignore_them() {
        let m = with_helpers();
        let ctx = Ctx::new(&m, &CtxOpts::default());
        // No region is named after a helper; it belongs to its driver.
        assert!(!ctx.regions.iter().any(|r| r.name.contains("_twist")));
        let w = band(&m, &ctx, &m.weights, 1.0);
        for h in &m.skel.helpers {
            let total: f64 = w.iter().flat_map(|vw| vw.iter()).filter(|e| e.0 as usize == h.joint).map(|e| e.1).sum();
            assert!(total > 1.0, "{} got no weight", m.skel.names[h.joint]);
        }
        for vw in &w {
            assert!(vw.len() <= 4);
            assert!((vw.iter().map(|e| e.1).sum::<f64>() - 1.0).abs() < 1e-9);
        }
        // Unweighted helpers do not change the check (noise is judged on motion).
        let plain = Model::from_glb(to_glb(&build("clean"))).unwrap();
        let a = evaluate(&m, &ctx, &m.weights);
        let b = evaluate(&plain, &Ctx::new(&plain, &CtxOpts::default()), &plain.weights);
        assert_eq!(a.flags, b.flags);
    }

    /// A crotch vertex weighted hips 0.4, left thigh 0.3, right thigh 0.3.
    /// Re-blending onto the thigh helpers must keep what the vertex does:
    /// as motion (`equivalent`) it is still 0.4 / 0.3 / 0.3, the same
    /// weights whichever helper runs first, and mirrored inputs give
    /// mirrored outputs.
    #[test]
    fn crotch_reblend_is_even_and_order_free() {
        let m = with_helpers();
        let sk = &m.skel;
        let j = |n: &str| sk.names.iter().position(|x| x == n).unwrap() as u32;
        let (hips, l, r) = (j("DEF-spine"), j("DEF-thigh.L"), j("DEF-thigh.R"));
        let (hl, hr) = (j("DEF-thigh_twist.L"), j("DEF-thigh_twist.R"));
        let swap = |x: u32| match x {
            _ if x == l => r,
            _ if x == r => l,
            _ if x == hl => hr,
            _ if x == hr => hl,
            o => o,
        };
        let sorted = |vw: &VW| {
            let mut v = vw.clone();
            v.sort_by_key(|e| e.0);
            v
        };
        let mut reversed = sk.clone();
        reversed.helpers.reverse();
        for (wh, wl, wr) in [(0.4, 0.3, 0.3), (0.5, 0.35, 0.15), (0.2, 0.1, 0.7)] {
            let input: Weights = vec![vec![(hips, wh), (l, wl), (r, wr)]];
            let mirrored: Weights = vec![vec![(hips, wh), (l, wr), (r, wl)]];
            let a = reassign(sk, &input);
            let b = reassign(&reversed, &input);
            for (x, y) in sorted(&a[0]).iter().zip(sorted(&b[0]).iter()) {
                assert_eq!(x.0, y.0);
                assert!((x.1 - y.1).abs() < 1e-12, "order changed the weights: {:?} vs {:?}", a[0], b[0]);
            }
            let motion = equivalent(sk, &a);
            let on = |j: u32| motion[0].iter().filter(|e| e.0 == j).map(|e| e.1).sum::<f64>();
            assert!((on(hips) - wh).abs() < 1e-9 && (on(l) - wl).abs() < 1e-9 && (on(r) - wr).abs() < 1e-9, "motion {:?}", motion[0]);
            let mut m_out: VW = reassign(sk, &mirrored)[0].iter().map(|&(j, x)| (swap(j), x)).collect();
            m_out.sort_by_key(|e| e.0);
            for (x, y) in sorted(&a[0]).iter().zip(m_out.iter()) {
                assert_eq!(x.0, y.0);
                assert!((x.1 - y.1).abs() < 1e-12, "mirror: {:?} vs {:?}", a[0], m_out);
            }
        }
    }

    #[test]
    fn names() {
        assert_eq!(by_name("DEF-upper_arm_twist.L").as_deref(), Some("DEF-upper_arm.L"));
        assert_eq!(by_name("DEF-thigh_twist.R").as_deref(), Some("DEF-thigh.R"));
        assert_eq!(by_name("DEF-upper_arm.L"), None);
        assert_eq!(by_name("upperarm_twist_01_l"), None);
    }

    #[test]
    fn reblend_keeps_mean_rotation() {
        for &(l, p) in &[(0.5, 0.5), (0.1, 0.9), (0.8, 0.2)] {
            let (s, h, d) = reblend(l, p, 0.5);
            let t = p * s + h + d;
            assert!((t - (l + p)).abs() < 1e-12);
            assert!(((h * 0.5 + d) / t - l / (l + p)).abs() < 1e-12);
        }
    }
}
