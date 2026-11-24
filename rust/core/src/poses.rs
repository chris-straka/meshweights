//! Range-of-motion pose sets.
//!
//! Moves are geometric, not per-rig Euler angles: "bend the forearm toward
//! forward by 140°" rotates the bone's rest direction toward the character's
//! forward axis, whatever the rig's local axis convention is. Twist turns a
//! bone about its own direction. Bones are found by anatomical role
//! (`forearm.L`, `spine*`...) resolved from common naming schemes (Rigify
//! DEF-, Mixamo, Unreal, plain), or by exact bone name.

use crate::math::{Quat, Vec3, v3};
use crate::scene::{Model, Skeleton};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub enum Dir {
    Up,
    Down,
    Forward,
    Back,
    /// Character's left (+X in glTF's +Z-forward convention).
    Left,
    Right,
    /// Away from the body midline (left for `.L` bones, right for `.R`).
    Out,
    In,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Motion {
    Bend(Dir, f64),
    Twist(f64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Move {
    /// Role (`forearm.L`, `spine*`, `fingers.R`) or exact bone name.
    pub bone: String,
    pub motion: Motion,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoseSpec {
    pub name: String,
    #[serde(default)]
    pub mirror: bool,
    pub moves: Vec<Move>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
    pub up: (f64, f64, f64),
    pub forward: (f64, f64, f64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PoseSet {
    pub name: String,
    #[serde(default)]
    pub frame: Option<Frame>,
    pub poses: Vec<PoseSpec>,
}

/// A resolved pose: world-space rotations at joints (axes in the rest
/// frame; children inherit their parent's motion), or local TRS overrides
/// sampled from an animation clip.
pub type LocalPose = (usize, Option<Vec3>, Option<Quat>, Option<Vec3>);
type NodeTrs = (Option<Vec3>, Option<Quat>, Option<Vec3>);

#[derive(Clone, Debug)]
pub struct Pose {
    pub name: String,
    pub moves: Vec<(usize, Vec3, f64)>,
    pub locals: Vec<LocalPose>,
}

pub const HUMANOID_RON: &str = include_str!("../../../poses/humanoid.ron");
pub const QUADRUPED_RON: &str = include_str!("../../../poses/quadruped.ron");
pub const WINGED_RON: &str = include_str!("../../../poses/winged.ron");
pub const MULTI_LIMB_RON: &str = include_str!("../../../poses/multi_limb.ron");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SkelClass {
    Humanoid,
    Quadruped,
    /// Legs, but not two-and-upright or four (a spider, a bird, a biped
    /// with a level spine).
    Legged,
    Custom,
}

impl SkelClass {
    pub fn as_str(self) -> &'static str {
        match self {
            SkelClass::Humanoid => "humanoid",
            SkelClass::Quadruped => "quadruped",
            SkelClass::Legged => "legged",
            SkelClass::Custom => "custom",
        }
    }
}

pub fn parse_pose_set(text: &str) -> Result<PoseSet, String> {
    let set: PoseSet = ron::from_str(text).map_err(|e| format!("pose set: {e}"))?;
    if set.poses.is_empty() {
        return Err("pose set has no poses".into());
    }
    if let Some(f) = &set.frame {
        let vals = [f.up.0, f.up.1, f.up.2, f.forward.0, f.forward.1, f.forward.2];
        if vals.iter().any(|x| !x.is_finite()) {
            return Err("pose set frame has a non-finite axis component".into());
        }
        let up = v3(f.up.0, f.up.1, f.up.2);
        let fwd = v3(f.forward.0, f.forward.1, f.forward.2);
        if up.len() <= 1e-9 || fwd.len() <= 1e-9 {
            return Err("pose set frame up/forward axis must be non-zero".into());
        }
        if up.normalized().cross(fwd.normalized()).len() <= 1e-6 {
            return Err("pose set frame up and forward axes must not be parallel".into());
        }
    }
    for p in &set.poses {
        for m in &p.moves {
            let angle = match m.motion {
                Motion::Bend(_, a) | Motion::Twist(a) => a,
            };
            if !angle.is_finite() {
                return Err(format!("pose '{}': angle is not finite ({angle})", p.name));
            }
        }
    }
    Ok(set)
}

// ---------------------------------------------------------------- roles

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    L,
    R,
    C,
}

fn strip_prefixes(s: &str) -> String {
    let mut s = s.to_lowercase();
    for p in ["def-", "def_", "mixamorig:", "mixamorig_", "mixamorig1:", "bip01 ", "bip001 ", "bip01_", "bip001_", "j_bip_", "j_"] {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest.to_string();
        }
    }
    if let Some(i) = s.rfind('|') {
        s = s[i + 1..].to_string();
    }
    s
}

fn side_of(n: &str) -> (Side, String) {
    let toks: Vec<&str> = n.split(['.', '_', '-', ' ', ':']).filter(|t| !t.is_empty()).collect();
    let mut side = Side::C;
    let mut rest = Vec::new();
    for t in &toks {
        match *t {
            "l" | "left" | "lft" => side = Side::L,
            "r" | "right" | "rgt" => side = Side::R,
            _ => rest.push(*t),
        }
    }
    let mut body = rest.join("_");
    if side == Side::C {
        for (pre, sd) in [("left", Side::L), ("right", Side::R)] {
            if let Some(i) = body.find(pre) {
                side = sd;
                body = format!("{}{}", &body[..i], &body[i + pre.len()..]);
                break;
            }
        }
    }
    (side, body)
}

fn part_of(body: &str) -> Option<&'static str> {
    let b = body.replace('_', "");
    let has = |w: &str| b.contains(w);
    if has("end") && (has("head") || has("top")) || has("nub") || has("twistend") {
        return None;
    }
    if has("thumb") || has("index") || has("middle") || has("ring") || has("pinky") || has("little") || has("finger") {
        return Some("fingers");
    }
    if has("forearm") || has("lowerarm") || has("elbow") {
        return Some("forearm");
    }
    if has("upperarm") || (has("arm") && !has("armature")) {
        return Some("upper_arm");
    }
    if has("hand") || has("wrist") {
        return Some("hand");
    }
    if has("shoulder") || has("clavicle") || has("collar") {
        return Some("shoulder");
    }
    if has("thigh") || has("upleg") || has("upperleg") {
        return Some("thigh");
    }
    if has("toe") {
        return Some("toe");
    }
    if has("foot") || has("ankle") {
        return Some("foot");
    }
    if has("shin") || has("calf") || has("lowerleg") || has("knee") || has("leg") {
        return Some("shin");
    }
    if has("neck") {
        return Some("neck");
    }
    if has("head") {
        return Some("head");
    }
    if has("spine") || has("chest") || has("torso") || has("abdomen") || has("hips") || has("pelvis") {
        return Some("spine");
    }
    None
}

/// Joints above `j` in the skeleton.
pub fn depth_of(sk: &Skeleton, j: usize) -> usize {
    depth(sk, j)
}

fn depth(sk: &Skeleton, j: usize) -> usize {
    let mut d = 0;
    let mut p = sk.jparent[j];
    while let Some(pj) = p {
        d += 1;
        p = sk.jparent[pj];
        if d > sk.joints.len() {
            break;
        }
    }
    d
}

/// Rigify's numbered spine (`spine`, `spine.001` ...), which the standard
/// humanoid skeleton (motionforge `standardize`) keeps: `spine` is the
/// hips, `.001`-`.003` the spine, `.004`-`.005` the neck, `.006` the head.
/// `None` for any other name.
fn rigify_spine(body: &str) -> Option<&'static str> {
    let n = body.strip_prefix("spine_")?;
    if n.len() != 3 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(match n.parse::<u32>().ok()? {
        1..=3 => "spine",
        4 | 5 => "neck",
        6 => "head",
        _ => "",
    })
}

/// Role -> joints, from the skeleton's structure (`anatomy.rs`).
/// Multi-bone roles: `spine*` (the chain above the hips to the chest),
/// `neck`, `fingers.L/R`, `tail*`.
pub fn resolve_roles(sk: &Skeleton) -> BTreeMap<String, Vec<usize>> {
    sk.anatomy.roles.clone()
}

/// The roles bone names alone would give (the checker's rule before
/// 2026-11-13: substring matches, `ring` in an eye-ring bone made it a
/// finger). Kept to compare against, never to pose by.
pub fn name_roles(sk: &Skeleton) -> BTreeMap<String, Vec<usize>> {
    let mut roles: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let posable = |j: usize| crate::helpers::helper_of(sk, j).is_none();
    // The numbered Rigify spine names its neck and head by position; a
    // skeleton that also has bones named neck or head goes by those names.
    let named_neck_head = (0..sk.joints.len()).filter(|&j| posable(j)).any(|j| {
        let (side, body) = side_of(&strip_prefixes(&sk.names[j]));
        side == Side::C && matches!(part_of(&body), Some("neck" | "head"))
    });
    for (j, name) in sk.names.iter().enumerate().take(sk.joints.len()) {
        // Helpers follow their driver; a pose never moves them directly.
        if !posable(j) {
            continue;
        }
        let (side, body) = side_of(&strip_prefixes(name));
        let part = match rigify_spine(&body) {
            Some(p) if !named_neck_head && side == Side::C => (!p.is_empty()).then_some(p),
            _ => part_of(&body),
        };
        let Some(part) = part else { continue };
        let key = match side {
            Side::L => format!("{part}.L"),
            Side::R => format!("{part}.R"),
            Side::C => part.to_string(),
        };
        roles.entry(key).or_default().push(j);
    }
    for v in roles.values_mut() {
        v.sort_by_key(|&j| (depth(sk, j), j));
    }
    // The spine chain: shallowest is the hips; the rest bend together.
    if let Some(sp) = roles.get("spine").cloned()
        && !sp.is_empty()
    {
        roles.insert("hips".into(), vec![sp[0]]);
        if sp.len() > 1
            && let Some(&chest) = sp.last()
        {
            roles.insert("spine*".into(), sp[1..].to_vec());
            roles.insert("chest".into(), vec![chest]);
        }
    }
    roles
}

pub fn classify(sk: &Skeleton) -> SkelClass {
    match sk.anatomy.body {
        crate::anatomy::Body::Humanoid => SkelClass::Humanoid,
        crate::anatomy::Body::Quadruped => SkelClass::Quadruped,
        crate::anatomy::Body::Legged => SkelClass::Legged,
        crate::anatomy::Body::Other => SkelClass::Custom,
    }
}

// ---------------------------------------------------------------- resolve

fn mirror_name(s: &str) -> String {
    if let Some(b) = s.strip_suffix(".L") {
        return format!("{b}.R");
    }
    if let Some(b) = s.strip_suffix(".R") {
        return format!("{b}.L");
    }
    s.to_string()
}

fn mirror_dir(d: Dir) -> Dir {
    match d {
        Dir::Left => Dir::Right,
        Dir::Right => Dir::Left,
        o => o,
    }
}

struct Basis {
    up: Vec3,
    fwd: Vec3,
    left: Vec3,
}

fn dir_vec(b: &Basis, d: Dir, side: Side) -> Vec3 {
    match d {
        Dir::Up => b.up,
        Dir::Down => -b.up,
        Dir::Forward => b.fwd,
        Dir::Back => -b.fwd,
        Dir::Left => b.left,
        Dir::Right => -b.left,
        Dir::Out => {
            if side == Side::R {
                -b.left
            } else {
                b.left
            }
        }
        Dir::In => {
            if side == Side::R {
                b.left
            } else {
                -b.left
            }
        }
    }
}

fn bone_dir(sk: &Skeleton, j: usize) -> Vec3 {
    (sk.tail[j] - sk.head[j]).normalized()
}

/// Expands a pose set against a skeleton. Moves on missing bones are
/// skipped; poses left with no moves are dropped (see `resolve_report`
/// for which, and why).
pub fn resolve(set: &PoseSet, sk: &Skeleton) -> Vec<Pose> {
    resolve_report(set, sk).0
}

/// `resolve`, plus one line per dropped pose saying what it could not
/// find, so a pose that cannot run is never left out without a word.
pub fn resolve_report(set: &PoseSet, sk: &Skeleton) -> (Vec<Pose>, Vec<String>) {
    let roles = resolve_roles(sk);
    let (up, fwd) = match &set.frame {
        Some(f) => (v3(f.up.0, f.up.1, f.up.2).normalized(), v3(f.forward.0, f.forward.1, f.forward.2).normalized()),
        // The facing the anatomy read from the feet (+Z when they say nothing).
        None if sk.anatomy.forward.len2() > 0.0 => (v3(0.0, 1.0, 0.0), sk.anatomy.forward),
        None => (v3(0.0, 1.0, 0.0), v3(0.0, 0.0, 1.0)),
    };
    let basis = Basis { up, fwd, left: up.cross(fwd).normalized() };
    let mut out = Vec::new();
    let mut specs: Vec<PoseSpec> = Vec::new();
    for p in &set.poses {
        if p.mirror {
            specs.push(PoseSpec { name: format!("{}.L", p.name), mirror: false, moves: p.moves.clone() });
            specs.push(PoseSpec {
                name: format!("{}.R", p.name),
                mirror: false,
                moves: p
                    .moves
                    .iter()
                    .map(|m| Move {
                        bone: mirror_name(&m.bone),
                        motion: match m.motion {
                            Motion::Bend(d, a) => Motion::Bend(mirror_dir(d), a),
                            Motion::Twist(a) => Motion::Twist(-a),
                        },
                    })
                    .collect(),
            });
        } else {
            specs.push(p.clone());
        }
    }
    let mut skipped = Vec::new();
    for p in specs {
        let mut moves = Vec::new();
        let mut missing: Vec<&str> = Vec::new();
        for m in &p.moves {
            let role_entry = roles.get(&m.bone).or_else(|| if m.bone == "neck*" { roles.get("neck") } else { None });
            let is_chain = crate::anatomy::is_chain_role(&m.bone);
            let (targets, side) = match role_entry {
                Some(js) => {
                    let side = if m.bone.ends_with(".R") {
                        Side::R
                    } else if m.bone.ends_with(".L") {
                        Side::L
                    } else {
                        Side::C
                    };
                    let js = if is_chain || m.bone.starts_with("fingers") { js.clone() } else { vec![js[0]] };
                    (js, side)
                }
                None => match sk.joint_by_name(&m.bone) {
                    Some(j) => (vec![j], side_of(&strip_prefixes(&m.bone)).0),
                    None => {
                        missing.push(&m.bone);
                        continue;
                    }
                },
            };
            // Multi-bone roles share the angle (a spine/wing/neck/tail bend
            // spreads over the chain); fingers each curl by the full angle.
            let share = if is_chain { targets.len() as f64 } else { 1.0 };
            for j in targets {
                let d = bone_dir(sk, j);
                if d.len2() == 0.0 {
                    continue;
                }
                match m.motion {
                    Motion::Twist(a) => moves.push((j, d, (a / share).to_radians())),
                    Motion::Bend(dir, a) => {
                        let t = dir_vec(&basis, dir, side);
                        let axis = d.cross(t);
                        if axis.len() < 0.15 {
                            continue; // bone already points that way
                        }
                        moves.push((j, axis.normalized(), (a / share).to_radians()));
                    }
                }
            }
        }
        if !moves.is_empty() {
            out.push(Pose { name: p.name, moves, locals: Vec::new() });
        } else if missing.is_empty() {
            skipped.push(format!("{}: every bone it moves already points that way", p.name));
        } else {
            skipped.push(format!("{}: no bone for {}", p.name, missing.join(", ")));
        }
    }
    (out, skipped)
}

const RIGID_CREATURE_TOKENS: &[&str] = &[
    "eye", "eyeball", "lid", "eyelid", "brow", "pupil", "iris", "horn", "antler", "tusk", "ear", "tooth", "teeth", "tongue", "whisker",
    "lip", "nose", "snout", "beak", "nostril", "claw", "nail", "hoof", "feather", "quill", "barb", "finlet", "gill", "antenna", "antennae",
];

fn rest_height(model: &Model) -> f64 {
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for p in &model.rest {
        lo = lo.min(p.y);
        hi = hi.max(p.y);
    }
    if hi > lo { hi - lo } else { 1.0 }
}

fn push_chain_curve(
    out: &mut Vec<Pose>,
    sk: &Skeleton,
    prefix: &str,
    chain: &[usize],
    bend_total_deg: f64,
    per_cap_deg: f64,
    twist_total_deg: f64,
) {
    let valid: Vec<(usize, Vec3)> = chain
        .iter()
        .copied()
        .filter_map(|j| {
            let d = bone_dir(sk, j);
            (d.len2() > 0.0).then_some((j, d))
        })
        .collect();
    if valid.is_empty() {
        return;
    }
    let len = valid.len() as f64;
    let d0 = valid[0].1;
    let a = d0.any_perp();
    let b = d0.cross(a).normalized();
    let per_bend = (bend_total_deg / len).min(per_cap_deg).to_radians();
    for (tag, axis, ang) in
        [("bend_a+", a, per_bend), ("bend_a-", a, -per_bend), ("bend_b+", b, per_bend * 0.85), ("bend_b-", b, -per_bend * 0.85)]
    {
        let moves: Vec<(usize, Vec3, f64)> = valid.iter().map(|&(j, _)| (j, axis, ang)).collect();
        out.push(Pose { name: format!("{prefix}_{tag}"), moves, locals: Vec::new() });
    }
    if valid.len() >= 3 && twist_total_deg > 0.0 {
        let per_tw = (twist_total_deg / len).min(8.0).to_radians();
        for (tag, sgn) in [("twist+", 1.0), ("twist-", -1.0)] {
            let moves: Vec<(usize, Vec3, f64)> = valid.iter().map(|&(j, d)| (j, d, sgn * per_tw)).collect();
            out.push(Pose { name: format!("{prefix}_{tag}"), moves, locals: Vec::new() });
        }
    }
}

/// Conservative ROM for non-humanoid skeletons: protects cranial/rigid
/// protrusions (`eye`, `horn`, `ear`, `beak`, feathers, tiny terminal stubs),
/// opens `jaw` downward only, and distributes bends smoothly along connected
/// chains (`angle / chain_len`) instead of applying ±45° per bone.
pub fn generic(model: &Model) -> Vec<Pose> {
    conservative_uncovered(model, &[])
}

fn conservative_uncovered(model: &Model, already_covered: &[bool]) -> Vec<Pose> {
    let sk = &model.skel;
    let n = sk.joints.len();
    let mut used = vec![0usize; n];
    let mut vert_mass = vec![0usize; n];
    let mut total_mass = 0usize;
    for vw in &model.weights {
        if let Some(&(j, w)) = vw.first()
            && w > 0.0
            && (j as usize) < n
        {
            vert_mass[j as usize] += 1;
            total_mass += 1;
        }
        for &(j, w) in vw {
            if w > 0.05 && (j as usize) < n {
                used[j as usize] += 1;
            }
        }
    }
    let mut territory = vec![0usize; n];
    for p in &model.rest {
        let j = (0..n).min_by(|&a, &b| sk.seg_dist(a, *p).total_cmp(&sk.seg_dist(b, *p))).unwrap_or(0);
        territory[j] += 1;
    }
    let min_t = (model.nverts() / 100).max(1);
    for j in 0..n {
        if territory[j] >= min_t {
            used[j] += 1;
        }
    }
    let h = rest_height(model);
    let roled: Vec<bool> = {
        let mut r = vec![false; n];
        for (role, js) in &sk.anatomy.roles {
            if role == "head" || role == "jaw" || role == "neck" || crate::anatomy::is_chain_role(role) {
                for &j in js {
                    r[j] = true;
                }
            }
        }
        r
    };
    let is_rigid_or_protected = |j: usize| -> bool {
        if already_covered.get(j).copied().unwrap_or(false) || crate::helpers::helper_of(sk, j).is_some() {
            return true;
        }
        // Protect children of `head` (eyes, horns, antlers, ears, beaks, teeth) unless explicitly neck/head/jaw.
        if sk.anatomy.group.get(j) == Some(&crate::anatomy::Group::Head) && !roled[j] {
            return true;
        }
        // Protect secondary wing/fin feather branches outside the main structural wing/fin chain.
        if let Some(crate::anatomy::Group::Limb(name)) = sk.anatomy.group.get(j)
            && (name.starts_with("wing") || name.starts_with("fin"))
            && !roled[j]
        {
            return true;
        }
        if crate::anatomy::has_token(&sk.names[j], RIGID_CREATURE_TOKENS) {
            return true;
        }
        let leaf = sk.jchildren[j].is_empty();
        let seg_len = (sk.tail[j] - sk.head[j]).len();
        leaf && total_mass > 0 && vert_mass[j].saturating_mul(60) < total_mass && seg_len < 0.03 * h
    };

    let mut out = Vec::new();
    // Gentle jaw opening (downward only, never upward through the skull).
    if let Some(jaw_js) = sk.anatomy.roles.get("jaw")
        && let Some(&jj) = jaw_js.first()
        && !already_covered.get(jj).copied().unwrap_or(false)
    {
        let d = bone_dir(sk, jj);
        if d.len2() > 0.0 {
            let fwd = if sk.anatomy.forward.len2() > 0.0 { sk.anatomy.forward } else { v3(0.0, 0.0, 1.0) };
            let up = v3(0.0, 1.0, 0.0);
            let axis = d.cross(-up);
            let ax = if axis.len() >= 0.15 { axis.normalized() } else { up.cross(fwd).normalized() };
            out.push(Pose { name: "jaw_open".into(), moves: vec![(jj, ax, 18f64.to_radians())], locals: Vec::new() });
        }
    }

    // Radial limbs (e.g. tentacles) get distributed curves + distal half curls.
    let mut handled = already_covered.to_vec();
    if handled.len() < n {
        handled.resize(n, false);
    }
    let mut limb_roles: Vec<(&String, &Vec<usize>)> =
        sk.anatomy.roles.iter().filter(|(r, _)| r.starts_with("limb") && crate::anatomy::is_chain_role(r)).collect();
    limb_roles.sort_by(|a, b| a.0.cmp(b.0));
    for (rname, chain) in limb_roles {
        let base = rname.trim_end_matches('*');
        if chain.iter().any(|&j| !handled[j]) {
            push_chain_curve(&mut out, sk, base, chain, 28.0, 16.0, 0.0);
            if chain.len() >= 4 {
                push_chain_curve(&mut out, sk, &format!("{base}_tip"), &chain[chain.len() / 2..], 20.0, 12.0, 0.0);
            }
            for &j in chain {
                handled[j] = true;
            }
        }
    }

    // Paired fins (on swimming creatures) get conservative sweeps without wing twists.
    for side in ["L", "R"] {
        let rname = format!("fin*.{side}");
        if let Some(chain) = sk.anatomy.roles.get(&rname)
            && chain.iter().any(|&j| !handled[j])
        {
            push_chain_curve(&mut out, sk, &format!("fin_{side}"), chain, 22.0, 14.0, 0.0);
            for &j in chain {
                handled[j] = true;
            }
        }
    }

    // Remaining uncovered active bones: group connected chains rather than per-bone +/-45 deg bends.
    for j in 0..n {
        let hierarchy_root = sk.jparent[j].is_none() && !sk.jchildren[j].is_empty();
        if handled[j] || hierarchy_root || (used[j] == 0 && sk.jchildren[j].iter().all(|&c| used[c] == 0)) || is_rigid_or_protected(j) {
            continue;
        }
        // Only start linear chains at chain heads so interior joints are curved smoothly as a chain.
        let parent_in_same_chain = sk.jparent[j].is_some_and(|p| {
            let p_root = sk.jparent[p].is_none() && !sk.jchildren[p].is_empty();
            !p_root && !handled[p] && !is_rigid_or_protected(p) && (used[p] > 0)
        });
        if parent_in_same_chain {
            continue;
        }
        let mut chain = vec![j];
        let mut cur = j;
        while let Some(&nxt) = sk.jchildren[cur]
            .iter()
            .filter(|&&c| !handled[c] && !is_rigid_or_protected(c) && (used[c] > 0 || sk.jchildren[c].iter().any(|&g| used[g] > 0)))
            .max_by_key(|&&c| (vert_mass[c] + used[c], std::cmp::Reverse(c)))
        {
            chain.push(nxt);
            cur = nxt;
            if chain.len() >= n {
                break;
            }
        }
        for &c in &chain {
            handled[c] = true;
        }
        if chain.len() >= 2 {
            let tag = &sk.names[j];
            push_chain_curve(&mut out, sk, tag, &chain, 26.0, 16.0, 18.0);
            if chain.len() >= 6 {
                let mid = chain.len() / 2;
                push_chain_curve(&mut out, sk, &format!("{tag}_prox"), &chain[..mid], 20.0, 12.0, 0.0);
                push_chain_curve(&mut out, sk, &format!("{tag}_dist"), &chain[mid..], 20.0, 12.0, 0.0);
            }
        } else {
            let d = bone_dir(sk, j);
            if d.len2() == 0.0 {
                continue;
            }
            let a = d.any_perp();
            let b = d.cross(a).normalized();
            let nm = &sk.names[j];
            let deg = 18f64.to_radians();
            for (tag, axis, ang) in [("bend_a+", a, deg), ("bend_a-", a, -deg), ("bend_b+", b, deg), ("bend_b-", b, -deg)] {
                out.push(Pose { name: format!("{nm}_{tag}"), moves: vec![(j, axis, ang)], locals: Vec::new() });
            }
        }
    }
    out
}

/// Names of every animation clip with TRS channels in the model, in file order.
pub fn clip_names(model: &Model) -> Vec<String> {
    model
        .glb
        .arr("animations")
        .iter()
        .enumerate()
        .filter_map(|(ai, anim)| {
            let samplers = anim.get("samplers").and_then(Value::as_array)?;
            let has_trs = anim.get("channels").and_then(Value::as_array)?.iter().any(|ch| {
                let Some(path) = ch.get("target").and_then(|t| t.get("path")).and_then(Value::as_str) else { return false };
                let Some(si) = ch.get("sampler").and_then(Value::as_u64) else { return false };
                ch.get("target").and_then(|t| t.get("node")).and_then(Value::as_u64).is_some()
                    && matches!(path, "translation" | "rotation" | "scale")
                    && samplers.get(si as usize).is_some()
            });
            has_trs.then(|| anim.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("clip{ai}")))
        })
        .collect()
}

/// Samples each animation clip at up to `per_clip` evenly spaced times.
pub fn clip_poses(model: &Model, per_clip: usize) -> Vec<Pose> {
    clip_sample(model, Sampling::Count(per_clip)).into_iter().flat_map(|c| c.1).collect()
}

/// Each animation clip, sampled every `1 / fps` seconds (its last key
/// included): (clip name, its frames).
pub fn clip_frames(model: &Model, fps: f64) -> Vec<(String, Vec<Pose>)> {
    clip_sample(model, Sampling::Fps(fps))
}

enum Sampling {
    Count(usize),
    Fps(f64),
}

fn clip_sample(model: &Model, how: Sampling) -> Vec<(String, Vec<Pose>)> {
    let glb = &model.glb;
    let mut out = Vec::new();
    for (ai, anim) in glb.arr("animations").iter().enumerate() {
        let name = anim.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| format!("clip{ai}"));
        let samplers = anim.get("samplers").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut tracks = Vec::new(); // (node, path, times, values, ncomp, step)
        let (mut t0, mut t1) = (f64::MAX, f64::MIN);
        for ch in anim.get("channels").and_then(Value::as_array).into_iter().flatten() {
            let (Some(node), Some(path), Some(si)) = (
                ch.get("target").and_then(|t| t.get("node")).and_then(Value::as_u64),
                ch.get("target").and_then(|t| t.get("path")).and_then(Value::as_str),
                ch.get("sampler").and_then(Value::as_u64),
            ) else {
                continue;
            };
            if !matches!(path, "translation" | "rotation" | "scale") {
                continue;
            }
            let Some(s) = samplers.get(si as usize) else { continue };
            let (Some(ia), Some(oa)) = (s.get("input").and_then(Value::as_u64), s.get("output").and_then(Value::as_u64)) else { continue };
            let Ok((_, times)) = glb.read_f64(ia as usize) else { continue };
            let Ok((nc, vals)) = glb.read_f64(oa as usize) else { continue };
            let interp = s.get("interpolation").and_then(Value::as_str).unwrap_or("LINEAR");
            if times.is_empty() || times.iter().any(|t| !t.is_finite()) || vals.iter().any(|v| !v.is_finite()) {
                continue;
            }
            // CUBICSPLINE stores (in, value, out) triplets: take the values.
            let vals: Vec<f64> = if interp == "CUBICSPLINE" {
                let stride = nc.saturating_mul(3);
                if stride == 0 || !vals.len().is_multiple_of(stride) {
                    continue;
                }
                vals.chunks_exact(stride).flat_map(|c| c[nc..2 * nc].iter().copied()).collect()
            } else {
                vals
            };
            if times.len().checked_mul(nc).is_none_or(|need| vals.len() < need) {
                continue;
            }
            if times.last().is_none_or(|t_last| !t0.min(times[0]).is_finite() || !t1.max(*t_last).is_finite()) {
                continue;
            }
            t0 = t0.min(times[0]);
            t1 = t1.max(*times.last().unwrap_or(&times[0]));
            tracks.push((node as usize, path.to_string(), times, vals, nc, interp == "STEP"));
        }
        if tracks.is_empty() || !t0.is_finite() || !t1.is_finite() || t1 < t0 {
            continue;
        }
        let times: Vec<f64> = match how {
            Sampling::Count(k) => {
                let k = k.max(1);
                (0..k).map(|s| if k == 1 { t0 } else { t0 + (t1 - t0) * s as f64 / (k - 1) as f64 }).collect()
            }
            Sampling::Fps(fps) => {
                if !fps.is_finite() || fps <= 0.0 {
                    continue;
                }
                let n = ((t1 - t0) * fps).floor().clamp(0.0, 100_000.0) as usize;
                let mut v: Vec<f64> = (0..=n).map(|i| t0 + i as f64 / fps).collect();
                if t1 - v.last().copied().unwrap_or(t0) > 0.25 / fps {
                    v.push(t1);
                }
                v
            }
        };
        let mut frames = Vec::new();
        for t in times {
            let mut locals: BTreeMap<usize, NodeTrs> = BTreeMap::new();
            for (node, path, times, vals, nc, step) in &tracks {
                let i = times.partition_point(|&x| x <= t).saturating_sub(1).min(times.len() - 1);
                let j = (i + 1).min(times.len() - 1);
                let f = if j == i || *step { 0.0 } else { ((t - times[i]) / (times[j] - times[i])).clamp(0.0, 1.0) };
                let a = &vals[i * nc..i * nc + nc];
                let b = &vals[j * nc..j * nc + nc];
                let e = locals.entry(*node).or_default();
                match path.as_str() {
                    "rotation" if *nc == 4 => {
                        let qa = Quat { x: a[0], y: a[1], z: a[2], w: a[3] }.normalized();
                        let qb = Quat { x: b[0], y: b[1], z: b[2], w: b[3] }.normalized();
                        e.1 = Some(qa.slerp(qb, f));
                    }
                    "translation" if *nc == 3 => e.0 = Some(v3(a[0], a[1], a[2]).lerp(v3(b[0], b[1], b[2]), f)),
                    "scale" if *nc == 3 => e.2 = Some(v3(a[0], a[1], a[2]).lerp(v3(b[0], b[1], b[2]), f)),
                    _ => {}
                }
            }
            frames.push(Pose {
                name: format!("{name}@{t:.2}s"),
                moves: Vec::new(),
                locals: locals.into_iter().map(|(n, (t, r, s))| (n, t, r, s)).collect(),
            });
        }
        out.push((name, frames));
    }
    out
}

/// Poses for a legged body that is neither a humanoid nor a quadruped:
/// each leg swings conservatively forward, back and up from its first joint
/// and bends at its middle one (`anatomy.rs` names them `leg<k>_upper.S`,
/// `leg<k>_lower.S`).
pub fn legged_set(sk: &Skeleton) -> PoseSet {
    let mut poses = Vec::new();
    for role in sk.anatomy.roles.keys() {
        let Some(base) = role.strip_suffix("_upper.L").or_else(|| role.strip_suffix("_upper.R")) else { continue };
        if !base.starts_with("leg") {
            continue;
        }
        let side = &role[role.len() - 1..];
        for (tag, d, a) in [("forward", Dir::Forward, 25.0), ("back", Dir::Back, 25.0), ("up", Dir::Up, 20.0)] {
            poses.push(PoseSpec {
                name: format!("{base}_{tag}.{side}"),
                mirror: false,
                moves: vec![Move { bone: role.clone(), motion: Motion::Bend(d, a) }],
            });
        }
        let lower = format!("{base}_lower.{side}");
        if sk.anatomy.roles.contains_key(&lower) {
            poses.push(PoseSpec {
                name: format!("{base}_bend.{side}"),
                mirror: false,
                moves: vec![Move { bone: lower, motion: Motion::Bend(Dir::In, 30.0) }],
            });
        }
    }
    PoseSet { name: "legged".into(), frame: None, poses }
}

/// Conservative opposite-phase creature gait and strike keyposes derived from
/// structural roles (`anatomy.rs`). Keeps left and right limbs in opposite
/// phases on `walk`/`trot` frames so blind planted swaps never depend solely on
/// symmetric lunges.
pub fn creature_gait_poses(model: &Model) -> Vec<Pose> {
    let sk = &model.skel;
    let roles = &sk.anatomy.roles;
    let mut specs = Vec::new();
    match sk.anatomy.body {
        crate::anatomy::Body::Humanoid => return Vec::new(),
        crate::anatomy::Body::Quadruped => {
            for (name, dir_l, dir_r) in [("gait_walk_L", 1.0, -1.0), ("gait_walk_R", -1.0, 1.0)] {
                let (fl, fr) = if dir_l > 0.0 { (Dir::Forward, Dir::Back) } else { (Dir::Back, Dir::Forward) };
                let spine_dir = if dir_l > 0.0 { Dir::Left } else { Dir::Right };
                let tail_dir = if dir_l > 0.0 { Dir::Right } else { Dir::Left };
                specs.push(PoseSpec {
                    name: name.into(),
                    mirror: false,
                    moves: vec![
                        Move { bone: "upper_arm.L".into(), motion: Motion::Bend(fl, 22.0) },
                        Move { bone: "upper_arm.R".into(), motion: Motion::Bend(fr, 20.0) },
                        Move { bone: "thigh.R".into(), motion: Motion::Bend(fl, 20.0) },
                        Move { bone: "thigh.L".into(), motion: Motion::Bend(fr, 18.0) },
                        Move { bone: "forearm.R".into(), motion: Motion::Bend(Dir::Back, 28.0) },
                        Move { bone: "shin.L".into(), motion: Motion::Bend(Dir::Back, 25.0) },
                        Move { bone: "spine*".into(), motion: Motion::Bend(spine_dir, 8.0) },
                        Move { bone: "tail*".into(), motion: Motion::Bend(tail_dir, 14.0) },
                    ],
                });
                let _ = dir_r;
            }
            specs.push(PoseSpec {
                name: "gait_trot_a".into(),
                mirror: false,
                moves: vec![
                    Move { bone: "upper_arm.L".into(), motion: Motion::Bend(Dir::Forward, 28.0) },
                    Move { bone: "upper_arm.R".into(), motion: Motion::Bend(Dir::Back, 24.0) },
                    Move { bone: "thigh.R".into(), motion: Motion::Bend(Dir::Forward, 26.0) },
                    Move { bone: "thigh.L".into(), motion: Motion::Bend(Dir::Back, 22.0) },
                    Move { bone: "forearm.L".into(), motion: Motion::Bend(Dir::Back, 20.0) },
                    Move { bone: "shin.R".into(), motion: Motion::Bend(Dir::Back, 20.0) },
                    Move { bone: "spine*".into(), motion: Motion::Bend(Dir::Left, 10.0) },
                    Move { bone: "tail*".into(), motion: Motion::Bend(Dir::Right, 16.0) },
                ],
            });
            specs.push(PoseSpec {
                name: "gait_lunge".into(),
                mirror: false,
                moves: vec![
                    Move { bone: "upper_arm.L".into(), motion: Motion::Bend(Dir::Back, 16.0) },
                    Move { bone: "upper_arm.R".into(), motion: Motion::Bend(Dir::Back, 16.0) },
                    Move { bone: "forearm.L".into(), motion: Motion::Bend(Dir::Back, 30.0) },
                    Move { bone: "forearm.R".into(), motion: Motion::Bend(Dir::Back, 30.0) },
                    Move { bone: "thigh.L".into(), motion: Motion::Bend(Dir::Forward, 20.0) },
                    Move { bone: "thigh.R".into(), motion: Motion::Bend(Dir::Forward, 20.0) },
                    Move { bone: "spine*".into(), motion: Motion::Bend(Dir::Down, 12.0) },
                    Move { bone: "neck*".into(), motion: Motion::Bend(Dir::Up, 15.0) },
                    Move { bone: "jaw".into(), motion: Motion::Bend(Dir::Down, 16.0) },
                ],
            });
            specs.push(PoseSpec {
                name: "gait_attack".into(),
                mirror: false,
                moves: vec![
                    Move { bone: "upper_arm.L".into(), motion: Motion::Bend(Dir::Forward, 32.0) },
                    Move { bone: "upper_arm.R".into(), motion: Motion::Bend(Dir::Back, 20.0) },
                    Move { bone: "spine*".into(), motion: Motion::Bend(Dir::Up, 12.0) },
                    Move { bone: "neck*".into(), motion: Motion::Bend(Dir::Forward, 18.0) },
                    Move { bone: "jaw".into(), motion: Motion::Bend(Dir::Down, 18.0) },
                ],
            });
        }
        crate::anatomy::Body::Legged => {
            let mut leg_roles: Vec<(usize, String, String)> = Vec::new();
            for role in roles.keys() {
                let Some(base) = role.strip_suffix("_upper.L").or_else(|| role.strip_suffix("_upper.R")) else { continue };
                if let Some(num_s) = base.strip_prefix("leg")
                    && let Ok(idx) = num_s.parse::<usize>()
                {
                    let side = &role[role.len() - 1..];
                    leg_roles.push((idx, side.to_string(), role.clone()));
                }
            }
            for (name, phase) in [("gait_walk_L", 0usize), ("gait_walk_R", 1usize)] {
                let mut moves = Vec::new();
                for (idx, side, role) in &leg_roles {
                    let fwd = (idx % 2 == phase) ^ (side == "R");
                    moves.push(Move { bone: role.clone(), motion: Motion::Bend(if fwd { Dir::Forward } else { Dir::Back }, 20.0) });
                    if fwd {
                        moves.push(Move { bone: role.clone(), motion: Motion::Bend(Dir::Up, 12.0) });
                    }
                }
                if roles.contains_key("wing*.L") && roles.contains_key("wing*.R") {
                    moves.push(Move { bone: "wing*.L".into(), motion: Motion::Bend(if phase == 0 { Dir::Up } else { Dir::Down }, 24.0) });
                    moves.push(Move { bone: "wing*.R".into(), motion: Motion::Bend(if phase == 0 { Dir::Down } else { Dir::Up }, 20.0) });
                }
                moves.push(Move { bone: "tail*".into(), motion: Motion::Bend(if phase == 0 { Dir::Left } else { Dir::Right }, 14.0) });
                specs.push(PoseSpec { name: name.into(), mirror: false, moves });
            }
            let mut lunge_moves = vec![
                Move { bone: "spine*".into(), motion: Motion::Bend(Dir::Down, 12.0) },
                Move { bone: "neck*".into(), motion: Motion::Bend(Dir::Forward, 15.0) },
                Move { bone: "jaw".into(), motion: Motion::Bend(Dir::Down, 16.0) },
            ];
            if roles.contains_key("wing*.L") && roles.contains_key("wing*.R") {
                lunge_moves.push(Move { bone: "wing*.L".into(), motion: Motion::Bend(Dir::Back, 20.0) });
                lunge_moves.push(Move { bone: "wing*.R".into(), motion: Motion::Bend(Dir::Back, 20.0) });
            }
            specs.push(PoseSpec { name: "gait_lunge".into(), mirror: false, moves: lunge_moves });
        }
        crate::anatomy::Body::Other => {
            for (name, s_dir, t_dir, f_l, f_r) in [
                ("swim_wave_L", Dir::Left, Dir::Right, Dir::Back, Dir::Forward),
                ("swim_wave_R", Dir::Right, Dir::Left, Dir::Forward, Dir::Back),
            ] {
                specs.push(PoseSpec {
                    name: name.into(),
                    mirror: false,
                    moves: vec![
                        Move { bone: "spine*".into(), motion: Motion::Bend(s_dir, 20.0) },
                        Move { bone: "tail*".into(), motion: Motion::Bend(t_dir, 26.0) },
                        Move { bone: "fin*.L".into(), motion: Motion::Bend(f_l, 18.0) },
                        Move { bone: "fin*.R".into(), motion: Motion::Bend(f_r, 18.0) },
                    ],
                });
            }
            specs.push(PoseSpec {
                name: "swim_lunge".into(),
                mirror: false,
                moves: vec![
                    Move { bone: "spine*".into(), motion: Motion::Bend(Dir::Down, 14.0) },
                    Move { bone: "tail*".into(), motion: Motion::Bend(Dir::Up, 18.0) },
                    Move { bone: "jaw".into(), motion: Motion::Bend(Dir::Down, 18.0) },
                ],
            });
        }
    }
    if specs.is_empty() {
        return Vec::new();
    }
    let set = PoseSet { name: "creature_gait".into(), frame: None, poses: specs };
    let (mut poses, _) = resolve_report(&set, sk);
    // Keep only poses where at least 2 bones move cleanly (or 1+ on axial creatures).
    poses.retain(|p| !p.moves.is_empty());
    poses
}

fn mark_covered(sk: &Skeleton, poses: &[Pose], include_limbs: bool) -> Vec<bool> {
    let n = sk.joints.len();
    let mut covered = vec![false; n];
    for p in poses {
        for &(j, _, _) in &p.moves {
            if j < n {
                covered[j] = true;
            }
        }
    }
    if include_limbs {
        for (j, g) in sk.anatomy.group.iter().enumerate() {
            if let crate::anatomy::Group::Limb(name) = g
                && !name.starts_with("limb")
                && !name.starts_with("fin")
            {
                covered[j] = true;
            }
        }
    }
    covered
}

/// The default pose set for a model: the class preset (or a user file),
/// plus samples of the asset's own clips. Also the poses of the set that
/// could not run on this skeleton, one line each.
pub fn default_poses(model: &Model, user: Option<&PoseSet>, clips: bool) -> (SkelClass, Vec<Pose>, Vec<String>) {
    let class = classify(&model.skel);
    let (mut poses, skipped) = match (user, class) {
        (Some(set), _) => resolve_report(set, &model.skel),
        (None, SkelClass::Humanoid) => resolve_report(&parse_pose_set(HUMANOID_RON).expect("built-in humanoid.ron"), &model.skel),
        (None, SkelClass::Quadruped) => {
            let (mut p, s) = resolve_report(&parse_pose_set(QUADRUPED_RON).expect("built-in quadruped.ron"), &model.skel);
            if model.skel.anatomy.roles.contains_key("wing*.L") && model.skel.anatomy.roles.contains_key("wing*.R") {
                let (wp, _) = resolve_report(&parse_pose_set(WINGED_RON).expect("built-in winged.ron"), &model.skel);
                p.extend(wp.into_iter().filter(|pose| pose.name.starts_with("wing_")));
            }
            let covered = mark_covered(&model.skel, &p, true);
            p.extend(conservative_uncovered(model, &covered));
            p.extend(creature_gait_poses(model));
            (p, s)
        }
        (None, SkelClass::Legged) => {
            let (mut p, s) = resolve_report(&legged_set(&model.skel), &model.skel);
            let has_wings = model.skel.anatomy.roles.contains_key("wing*.L") && model.skel.anatomy.roles.contains_key("wing*.R");
            let ron = if has_wings { WINGED_RON } else { MULTI_LIMB_RON };
            let (cp, _) = resolve_report(&parse_pose_set(ron).expect("built-in creature ROM"), &model.skel);
            p.extend(cp);
            let covered = mark_covered(&model.skel, &p, true);
            p.extend(conservative_uncovered(model, &covered));
            p.extend(creature_gait_poses(model));
            (p, s)
        }
        (None, SkelClass::Custom) => {
            let (mut p, _) = resolve_report(&parse_pose_set(MULTI_LIMB_RON).expect("built-in multi_limb.ron"), &model.skel);
            if model.skel.anatomy.roles.contains_key("wing*.L") && model.skel.anatomy.roles.contains_key("wing*.R") {
                let (wp, _) = resolve_report(&parse_pose_set(WINGED_RON).expect("built-in winged.ron"), &model.skel);
                p.extend(wp.into_iter().filter(|pose| pose.name.starts_with("wing_")));
            }
            let covered = mark_covered(&model.skel, &p, false);
            p.extend(conservative_uncovered(model, &covered));
            p.extend(creature_gait_poses(model));
            (p, Vec::new())
        }
    };
    if clips {
        poses.extend(clip_poses(model, 6));
    }
    (class, poses, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn role_names() {
        let cases = [
            ("DEF-forearm.L", "forearm.L"),
            ("DEF-upper_arm.R", "upper_arm.R"),
            ("mixamorig:LeftForeArm", "forearm.L"),
            ("mixamorig:RightUpLeg", "thigh.R"),
            ("mixamorig:LeftLeg", "shin.L"),
            ("calf_l", "shin.L"),
            ("lowerarm_r", "forearm.R"),
            ("L_Upperarm", "upper_arm.L"),
            ("DEF-thigh.L", "thigh.L"),
            ("DEF-shin.R", "shin.R"),
            ("mixamorig:LeftHandIndex1", "fingers.L"),
            ("DEF-spine.003", "spine"),
            ("Head", "head"),
            ("mixamorig:Neck", "neck"),
            ("mixamorig:Spine2", "spine"),
        ];
        for (n, want) in cases {
            let (side, body) = side_of(&strip_prefixes(n));
            let part = part_of(&body).unwrap_or("?");
            let got = match side {
                Side::L => format!("{part}.L"),
                Side::R => format!("{part}.R"),
                Side::C => part.to_string(),
            };
            assert_eq!(got, want, "{n}");
        }
    }
    #[test]
    fn rigify_numbered_spine() {
        for (n, want) in [
            ("spine_001", Some("spine")),
            ("spine_003", Some("spine")),
            ("spine_004", Some("neck")),
            ("spine_005", Some("neck")),
            ("spine_006", Some("head")),
            ("spine_007", Some("")),
            ("spine", None),
            ("spine2", None),
            ("spine_02", None),
        ] {
            assert_eq!(rigify_spine(n), want, "{n}");
        }
        assert_eq!(side_of(&strip_prefixes("DEF-spine.004")).1, "spine_004");
    }

    #[test]
    fn humanoid_ron_parses() {
        let s = parse_pose_set(HUMANOID_RON).unwrap();
        assert!(s.poses.len() >= 10);
    }

    #[test]
    fn cubicspline_short_triplet_and_bad_fps_do_not_panic() {
        let mut g = crate::fixtures::to_glb(&crate::fixtures::build("clean"));
        let bv_t = g.append_view(&0.0f32.to_le_bytes(), None).unwrap();
        let acc_t = g.push_accessor(serde_json::json!({"bufferView": bv_t, "componentType": 5126, "count": 1, "type": "SCALAR"})).unwrap();
        let mut vbytes = Vec::new();
        for v in [0.0f32, 0.0, 0.0] {
            vbytes.extend_from_slice(&v.to_le_bytes());
        }
        let bv_v = g.append_view(&vbytes, None).unwrap();
        // 1 VEC3 instead of 3*1 VEC3 triplets expected by CUBICSPLINE.
        let acc_v = g.push_accessor(serde_json::json!({"bufferView": bv_v, "componentType": 5126, "count": 1, "type": "VEC3"})).unwrap();
        g.json["animations"] = serde_json::json!([{
            "name": "bad_cubic",
            "samplers": [{"input": acc_t, "output": acc_v, "interpolation": "CUBICSPLINE"}],
            "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}]
        }]);
        g.sync_buffer_len().unwrap();
        let m = Model::from_glb(g).unwrap();
        assert!(clip_poses(&m, 4).is_empty());
        assert!(clip_frames(&m, -1.0).is_empty());
    }

    #[test]
    fn parse_pose_set_rejects_empty_and_invalid_frames_or_angles() {
        assert!(parse_pose_set(r#"PoseSet(name: "empty", poses: [])"#).is_err());
        assert!(
            parse_pose_set(
                r#"PoseSet(name: "zero_frame", frame: Some((up: (0.0, 0.0, 0.0), forward: (0.0, 0.0, 1.0))), poses: [(name: "p", moves: [(bone: "head", motion: Twist(10.0))])])"#,
            )
            .is_err()
        );
        assert!(
            parse_pose_set(
                r#"PoseSet(name: "collinear", frame: Some((up: (0.0, 1.0, 0.0), forward: (0.0, 2.0, 0.0))), poses: [(name: "p", moves: [(bone: "head", motion: Twist(10.0))])])"#,
            )
            .is_err()
        );
        assert!(parse_pose_set(r#"PoseSet(name: "nan_ang", poses: [(name: "p", moves: [(bone: "head", motion: Twist(NaN))])])"#).is_err());
    }

    #[test]
    fn creature_ron_presets_parse_and_share_chain_angles() {
        let q = parse_pose_set(QUADRUPED_RON).unwrap();
        let w = parse_pose_set(WINGED_RON).unwrap();
        let ml = parse_pose_set(MULTI_LIMB_RON).unwrap();
        assert!(q.poses.len() >= 18);
        assert!(w.poses.len() >= 12);
        assert!(ml.poses.len() >= 8);
    }
}
