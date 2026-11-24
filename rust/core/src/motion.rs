//! The game's own motion on any rig: clips sampled at a game's frame rate,
//! from the rig's own file or transferred from another rig's (the hero's
//! walk, run, attack, dodge, hit, death).
//!
//! Transfer is by role (`anatomy.rs`), so it needs no bone names: per
//! frame, each source bone's world rotation change from rest is applied
//! to the target bone of the same role, after turning the target bone so
//! its rest direction matches the source's (a clip made on an A-pose rig
//! keeps its arm angles on a T-pose rig) and after a yaw when the two face
//! different ways. This is motionforge `animate`'s transfer (`d_t = C d
//! C^-1` with rest alignment), done here only to measure: nothing is
//! written, and root travel and the hips' height are left out (the checks
//! are about the body's shape, not where it stands). motionforge
//! `animate` stays the way clips reach the game; a rig it has animated
//! carries its clips, and those are measured as they are (contact pass
//! and all).

use crate::anatomy::Body;
use crate::math::{Quat, Vec3, v3};
use crate::poses::Pose;
use crate::scene::Model;
use std::collections::BTreeMap;

/// The hero's clips the checker plays by default (the ones the owner
/// watches a character in).
pub const GAME_CLIPS: &[&str] = &["walk", "run", "attack_1", "dodge", "hit", "death"];

/// Frames per second clips are sampled at.
pub const FPS: f64 = 15.0;

pub struct Motion {
    pub poses: Vec<Pose>,
    /// Clips played, in order.
    pub clips: Vec<String>,
    /// Where they came from: "own" (the file's) or the source file's name.
    pub source: String,
    /// One line per clip asked for that could not play, and why.
    pub skipped: Vec<String>,
}

/// Every clip of the model's own file, sampled at `fps` (`names`: only
/// those, in that order).
pub fn own(model: &Model, fps: f64, names: Option<&[String]>) -> Motion {
    let all = crate::poses::clip_frames(model, fps);
    let mut out = Motion { poses: Vec::new(), clips: Vec::new(), source: "own".into(), skipped: Vec::new() };
    let wanted: Vec<String> = match names {
        Some(n) => n.to_vec(),
        None => all.iter().map(|c| c.0.clone()).collect(),
    };
    for w in &wanted {
        match all.iter().find(|c| &c.0 == w) {
            Some((name, poses)) => {
                out.clips.push(name.clone());
                out.poses.extend(poses.iter().cloned());
            }
            None => out.skipped.push(format!("{w}: not in the file")),
        }
    }
    out
}

/// The angle and axis of a unit quaternion (axis unit, angle in [0, pi]).
fn axis_angle(q: Quat) -> (Vec3, f64) {
    let q = if q.w < 0.0 { Quat { x: -q.x, y: -q.y, z: -q.z, w: -q.w } } else { q };
    let s = (q.x * q.x + q.y * q.y + q.z * q.z).sqrt();
    if s < 1e-12 {
        return (v3(1.0, 0.0, 0.0), 0.0);
    }
    (v3(q.x / s, q.y / s, q.z / s), 2.0 * s.atan2(q.w))
}

/// Source joint per target joint, by role. Chains (`spine*`, `neck`)
/// map by their place along the chain.
fn role_map(src: &Model, dst: &Model) -> Vec<(usize, usize)> {
    let (rs, rd) = (&src.skel.anatomy.roles, &dst.skel.anatomy.roles);
    let mut out: BTreeMap<usize, usize> = BTreeMap::new();
    for (role, dj) in rd {
        let Some(sj) = rs.get(role) else { continue };
        if role.starts_with("fingers") || sj.is_empty() || dj.is_empty() {
            continue;
        }
        if crate::anatomy::is_chain_role(role) || role == "neck" {
            let (m, k) = (dj.len(), sj.len());
            for (i, &d) in dj.iter().enumerate() {
                let f = (i + 1) as f64 / m as f64;
                let si = ((f * k as f64).round() as usize).clamp(1, k) - 1;
                out.entry(d).or_insert(sj[si]);
            }
        } else {
            out.entry(dj[0]).or_insert(sj[0]);
        }
    }
    out.into_iter().collect()
}

fn dir(m: &Model, j: usize) -> Option<Vec3> {
    let d = m.skel.tail[j] - m.skel.head[j];
    (d.len() > 1e-9).then(|| d.normalized())
}

fn has_real_tail(m: &Model, j: usize) -> bool {
    m.skel.jchildren[j].iter().any(|&c| (m.skel.head[c] - m.skel.tail[j]).len() < 1e-6)
}

/// Direct joint-to-joint map when `src` and `dst` share the same skeleton
/// (same joint names and parent links, and matching rest directions on bones
/// that have a real child at their tail).
fn same_skeleton_map(src: &Model, dst: &Model) -> Option<Vec<(usize, usize)>> {
    let n = dst.njoints();
    if n == 0 || src.njoints() != n {
        return None;
    }
    let mut by_name: BTreeMap<&str, usize> = BTreeMap::new();
    for (s, name) in src.skel.names.iter().enumerate() {
        if by_name.insert(name.as_str(), s).is_some() {
            return None;
        }
    }
    let mut map = Vec::with_capacity(n);
    for (d, name) in dst.skel.names.iter().enumerate() {
        let &s = by_name.get(name.as_str())?;
        let dp = dst.skel.jparent[d].map(|p| dst.skel.names[p].as_str());
        let sp = src.skel.jparent[s].map(|p| src.skel.names[p].as_str());
        if dp != sp {
            return None;
        }
        map.push((d, s));
    }
    for &(d, s) in &map {
        if has_real_tail(dst, d)
            && has_real_tail(src, s)
            && let (Some(dd), Some(sd)) = (dir(dst, d), dir(src, s))
            && dd.dot(sd) < 0.995
        {
            return None;
        }
    }
    Some(map)
}

/// The source's clips on `dst`, sampled at `fps`. `names`: the clips to
/// play (those missing are listed in `skipped`).
pub fn transfer(src: &Model, src_name: &str, dst: &Model, names: &[String], fps: f64) -> Result<Motion, String> {
    let (map, yaw, align) = if let Some(same) = same_skeleton_map(src, dst) {
        let align: BTreeMap<usize, Quat> = same.iter().map(|&(d, _)| (d, Quat::IDENTITY)).collect();
        (same, Quat::IDENTITY, align)
    } else {
        let (bs, bd) = (src.skel.anatomy.body, dst.skel.anatomy.body);
        if bs != bd {
            return Err(format!("the clips are for a {} body and this is a {} one", bs.as_str(), bd.as_str()));
        }
        let map = role_map(src, dst);
        let min_bones = if bd == Body::Humanoid { 8 } else { 4 };
        if map.len() < min_bones {
            return Err(format!("only {} bones in common by role", map.len()));
        }
        // The yaw about up (a 180-degree turn from from_to could tip the body
        // over; review 2026-11-13).
        let (fs, fd) = (src.skel.anatomy.forward, dst.skel.anatomy.forward);
        let ang = fd.z.atan2(fd.x) - fs.z.atan2(fs.x);
        let yaw = Quat::from_axis_angle(v3(0.0, 1.0, 0.0), -ang);
        // Rest alignment for limbs (the spine and hips keep the target's own
        // rest shape).
        let limb = |role: &str| {
            [
                "shoulder",
                "upper_arm",
                "forearm",
                "hand",
                "thigh",
                "shin",
                "foot",
                "toe",
                "neck",
                "head",
                "leg",
                "tail",
                "wing",
                "fin",
                "jaw",
                "limb",
            ]
            .iter()
            .any(|r| role.starts_with(r))
        };
        let role_of: BTreeMap<usize, String> =
            dst.skel.anatomy.roles.iter().flat_map(|(r, js)| js.iter().map(move |&j| (j, r.clone()))).collect();
        let align: BTreeMap<usize, Quat> = map
            .iter()
            .map(|&(d, s)| {
                // Only bones whose direction both rigs give (a child at the
                // tail); an end bone's tail is a guess, and aligning to it bent
                // feet and hands 70-100 degrees at rest (review 2026-11-13).
                let a = match (role_of.get(&d), dir(dst, d), dir(src, s)) {
                    (Some(r), Some(dd), Some(sd)) if limb(r) && has_real_tail(dst, d) && has_real_tail(src, s) => {
                        Quat::from_to(dd, yaw.rotate(sd))
                    }
                    _ => Quat::IDENTITY,
                };
                (d, a)
            })
            .collect();
        (map, yaw, align)
    };
    let src_of: BTreeMap<usize, usize> = map.iter().copied().collect();
    // Target joints in parent-first order.
    let depth = |j: usize| {
        let mut d = 0;
        let mut p = dst.skel.jparent[j];
        while let Some(q) = p {
            d += 1;
            p = dst.skel.jparent[q];
            if d > dst.njoints() {
                break;
            }
        }
        d
    };
    let mut order: Vec<usize> = src_of.keys().copied().collect();
    order.sort_by_key(|&j| (depth(j), j));
    let frames = crate::poses::clip_frames(src, fps);
    let mut out = Motion { poses: Vec::new(), clips: Vec::new(), source: src_name.to_string(), skipped: Vec::new() };
    for want in names {
        let Some((clip, poses)) = frames.iter().find(|c| &c.0 == want) else {
            out.skipped.push(format!("{want}: not in {src_name}"));
            continue;
        };
        out.clips.push(clip.clone());
        for p in poses {
            let ms = crate::skin::joint_matrices(src, p);
            let mut world: BTreeMap<usize, Quat> = BTreeMap::new();
            let mut moves = Vec::new();
            for &d in &order {
                let s = src_of[&d];
                let ds = ms[s].rotation().normalized();
                let w = (yaw * ds * yaw.conj() * align[&d]).normalized();
                // Nearest mapped ancestor's world change.
                let mut pa = dst.skel.jparent[d];
                let parent = loop {
                    match pa {
                        Some(q) if world.contains_key(&q) => break world[&q],
                        Some(q) => pa = dst.skel.jparent[q],
                        None => break Quat::IDENTITY,
                    }
                };
                world.insert(d, w);
                let (axis, ang) = axis_angle((parent.conj() * w).normalized());
                if ang > 1e-7 {
                    moves.push((d, axis, ang));
                }
            }
            out.poses.push(Pose { name: p.name.clone(), moves, locals: Vec::new() });
        }
    }
    Ok(out)
}

/// Neutral procedural locomotion clips (`walk`, `trot`, `lunge`, `attack`) for
/// non-humanoid rigs whose GLB carries no animations, keyed from structural
/// anatomy (`quadruped`, `winged`, `multi_limb`, or axial swimming/serpentine).
pub fn creature_gait_clips(model: &Model, fps: f64) -> Motion {
    let mut out = Motion { poses: Vec::new(), clips: Vec::new(), source: "creature-gait".into(), skipped: Vec::new() };
    let sk = &model.skel;
    if sk.anatomy.body == Body::Humanoid || !fps.is_finite() || fps <= 0.0 {
        return out;
    }
    let up = v3(0.0, 1.0, 0.0);
    let fwd = if sk.anatomy.forward.len2() > 0.0 { sk.anatomy.forward } else { v3(0.0, 0.0, 1.0) };
    let left = up.cross(fwd).normalized();
    let roles = &sk.anatomy.roles;

    let bend_to = |j: usize, target: Vec3, deg: f64| -> Option<(usize, Vec3, f64)> {
        let d = (sk.tail[j] - sk.head[j]).normalized();
        if d.len2() == 0.0 {
            return None;
        }
        let ax = d.cross(target);
        if ax.len() < 0.15 { None } else { Some((j, ax.normalized(), deg.to_radians())) }
    };
    let push_chain = |moves: &mut Vec<(usize, Vec3, f64)>, role: &str, target: Vec3, total_deg: f64| {
        let Some(js) = roles.get(role) else { return };
        let share = js.len().max(1) as f64;
        for &j in js {
            if let Some(m) = bend_to(j, target, total_deg / share) {
                moves.push(m);
            }
        }
    };
    let push_head_role = |moves: &mut Vec<(usize, Vec3, f64)>, role: &str, target: Vec3, total_deg: f64| {
        if let Some(js) = roles.get(role)
            && let Some(&j) = js.first()
            && let Some(m) = bend_to(j, target, total_deg)
        {
            moves.push(m);
        }
    };

    let sample_cycle = |clip: &str, dur: f64, f: &dyn Fn(f64) -> Vec<(usize, Vec3, f64)>, out: &mut Motion| {
        let n = ((dur * fps).floor().clamp(2.0, 120.0)) as usize;
        let mut added = 0usize;
        for i in 0..=n {
            let t = dur * i as f64 / n as f64;
            let ph = i as f64 / n as f64;
            let moves = f(ph);
            if !moves.is_empty() {
                out.poses.push(Pose { name: format!("{clip}@{t:.2}s"), moves, locals: Vec::new() });
                added += 1;
            }
        }
        if added > 0 {
            out.clips.push(clip.into());
        }
    };

    let multi_legs: Vec<(usize, String, String)> = roles
        .keys()
        .filter_map(|r| {
            let base = r.strip_suffix("_upper.L").or_else(|| r.strip_suffix("_upper.R"))?;
            let idx = base.strip_prefix("leg")?.parse::<usize>().ok()?;
            let side = r[r.len() - 1..].to_string();
            Some((idx, side, r.clone()))
        })
        .collect();

    for (clip, dur, stride_deg, spine_deg) in [("walk", 0.8, 22.0, 8.0), ("trot", 0.6, 28.0, 12.0)] {
        sample_cycle(
            clip,
            dur,
            &|ph| {
                let s = (std::f64::consts::TAU * ph).sin();
                let mut moves = Vec::new();
                match sk.anatomy.body {
                    Body::Quadruped => {
                        push_head_role(&mut moves, "upper_arm.L", fwd, stride_deg * s);
                        push_head_role(&mut moves, "upper_arm.R", fwd, -stride_deg * s);
                        push_head_role(&mut moves, "thigh.R", fwd, 0.9 * stride_deg * s);
                        push_head_role(&mut moves, "thigh.L", fwd, -0.9 * stride_deg * s);
                        if s < 0.0 {
                            push_head_role(&mut moves, "forearm.L", -fwd, 28.0 * (-s));
                            push_head_role(&mut moves, "shin.R", -fwd, 25.0 * (-s));
                        } else {
                            push_head_role(&mut moves, "forearm.R", -fwd, 28.0 * s);
                            push_head_role(&mut moves, "shin.L", -fwd, 25.0 * s);
                        }
                        push_chain(&mut moves, "spine*", left, spine_deg * s);
                        push_chain(&mut moves, "tail*", -left, 14.0 * s);
                    }
                    Body::Legged => {
                        for (idx, side, role) in &multi_legs {
                            let alt = if (idx % 2 == 0) ^ (side == "R") { 1.0 } else { -1.0 };
                            push_head_role(&mut moves, role, fwd, stride_deg * 0.75 * alt * s);
                        }
                        push_chain(&mut moves, "wing*.L", up, 24.0 * s);
                        push_chain(&mut moves, "wing*.R", up, -20.0 * s);
                        push_chain(&mut moves, "spine*", left, spine_deg * s);
                        push_chain(&mut moves, "tail*", -left, 12.0 * s);
                    }
                    Body::Other => {
                        push_chain(&mut moves, "spine*", left, 18.0 * s);
                        push_chain(&mut moves, "tail*", -left, 24.0 * s);
                        push_chain(&mut moves, "fin*.L", -fwd, 16.0 * s);
                        push_chain(&mut moves, "fin*.R", -fwd, -16.0 * s);
                        for (r, js) in roles {
                            if r.starts_with("limb") && crate::anatomy::is_chain_role(r) {
                                let alt = if r.as_bytes().get(4).is_some_and(|b| b % 2 == 0) { 1.0 } else { -1.0 };
                                let share = js.len().max(1) as f64;
                                for &j in js {
                                    if let Some(m) = bend_to(j, up, 22.0 * alt * s / share) {
                                        moves.push(m);
                                    }
                                }
                            }
                        }
                    }
                    Body::Humanoid => {}
                }
                moves
            },
            &mut out,
        );
    }

    sample_cycle(
        "lunge",
        0.5,
        &|ph| {
            let p = (std::f64::consts::PI * ph).sin();
            let mut moves = Vec::new();
            match sk.anatomy.body {
                Body::Quadruped => {
                    push_head_role(&mut moves, "upper_arm.L", -fwd, 16.0 * p);
                    push_head_role(&mut moves, "upper_arm.R", -fwd, 16.0 * p);
                    push_head_role(&mut moves, "forearm.L", -fwd, 30.0 * p);
                    push_head_role(&mut moves, "forearm.R", -fwd, 30.0 * p);
                    push_head_role(&mut moves, "thigh.L", fwd, 20.0 * p);
                    push_head_role(&mut moves, "thigh.R", fwd, 20.0 * p);
                }
                Body::Legged => {
                    push_chain(&mut moves, "wing*.L", -fwd, 22.0 * p);
                    push_chain(&mut moves, "wing*.R", -fwd, 22.0 * p);
                }
                Body::Other => {
                    push_chain(&mut moves, "tail*", up, 18.0 * p);
                }
                Body::Humanoid => {}
            }
            push_chain(&mut moves, "spine*", -up, 12.0 * p);
            push_chain(&mut moves, "neck", up, 14.0 * p);
            push_head_role(&mut moves, "jaw", -up, 16.0 * p);
            moves
        },
        &mut out,
    );

    sample_cycle(
        "attack",
        0.5,
        &|ph| {
            let p = (std::f64::consts::PI * ph).sin();
            let mut moves = Vec::new();
            match sk.anatomy.body {
                Body::Quadruped => {
                    push_head_role(&mut moves, "upper_arm.L", fwd, 32.0 * p);
                    push_head_role(&mut moves, "upper_arm.R", -fwd, 18.0 * p);
                    push_chain(&mut moves, "spine*", up, 12.0 * p);
                }
                Body::Legged => {
                    if let Some((_, _, first)) = multi_legs.first() {
                        push_head_role(&mut moves, first, fwd, 26.0 * p);
                    }
                    push_chain(&mut moves, "wing*.L", up, 26.0 * p);
                    push_chain(&mut moves, "wing*.R", up, 26.0 * p);
                }
                Body::Other => {
                    push_chain(&mut moves, "spine*", fwd, 14.0 * p);
                }
                Body::Humanoid => {}
            }
            push_chain(&mut moves, "neck", fwd, 18.0 * p);
            push_head_role(&mut moves, "jaw", -up, 18.0 * p);
            moves
        },
        &mut out,
    );

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    /// A pose keyed on the mannequin's own nodes, transferred onto the
    /// same mannequin with every bone renamed: every bone ends up turned
    /// the same way, relative to the hips.
    #[test]
    fn transfer_onto_the_same_body_repeats_the_pose() {
        let src = crate::scene::Model::from_glb(fixtures::to_glb(&fixtures::build("clean"))).unwrap();
        let mut f = fixtures::build("clean");
        for (i, b) in f.bones.iter_mut().enumerate() {
            b.name = Box::leak(format!("bone_{i}").into_boxed_str());
        }
        let dst = crate::scene::Model::from_glb(fixtures::to_glb(&f)).unwrap();
        let sk = &src.skel;
        let key = |n: &str, axis: Vec3, deg: f64| {
            let j = sk.joint_by_name(n).unwrap();
            let node = sk.joints[j];
            (node, None, Some((Quat::from_axis_angle(axis, deg.to_radians()) * sk.nodes[node].r).normalized()), None)
        };
        let pose = Pose {
            name: "t".into(),
            moves: Vec::new(),
            locals: vec![
                key("DEF-forearm.L", v3(1.0, 0.0, 0.0), 50.0),
                key("DEF-thigh.R", v3(1.0, 0.0, 0.0), -35.0),
                key("DEF-spine.002", v3(0.0, 1.0, 0.0), 20.0),
                key("DEF-head", v3(0.0, 0.0, 1.0), 15.0),
            ],
        };
        let ms = crate::skin::joint_matrices(&src, &pose);
        // One "clip" of that pose, through the role map.
        let map = role_map(&src, &dst);
        assert!(map.len() >= 14, "{map:?}");
        let mut world: BTreeMap<usize, Quat> = BTreeMap::new();
        let mut moves = Vec::new();
        let mut order: Vec<(usize, usize)> = map.clone();
        order.sort_by_key(|&(d, _)| (crate::poses::depth_of(&dst.skel, d), d));
        for &(d, s) in &order {
            let w = ms[s].rotation().normalized();
            let mut pa = dst.skel.jparent[d];
            let parent = loop {
                match pa {
                    Some(q) if world.contains_key(&q) => break world[&q],
                    Some(q) => pa = dst.skel.jparent[q],
                    None => break Quat::IDENTITY,
                }
            };
            world.insert(d, w);
            let (axis, ang) = axis_angle((parent.conj() * w).normalized());
            moves.push((d, axis, ang));
        }
        let md = crate::skin::joint_matrices(&dst, &Pose { name: "t".into(), moves, locals: Vec::new() });
        for &(d, s) in &map {
            let (a, b) = (md[d].rotation(), ms[s].rotation());
            assert!(a.dot(b).abs() > 1.0 - 1e-9, "{}: {a:?} vs {b:?}", dst.skel.names[d]);
            let ha = md[d].transform_point(dst.skel.head[d]);
            let hb = ms[s].transform_point(src.skel.head[s]);
            assert!((ha - hb).len() < 1e-9, "{}", dst.skel.names[d]);
        }
    }
}
