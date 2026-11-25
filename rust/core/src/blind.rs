//! Same-frame blind pairs at the game camera, for judging a rig claim
//! by eye instead of by a number.
//!
//! A *claim* is one failing headline finding (`owner.rs`): "this skin
//! tears", "that part follows the wrong bone". For each claim this
//! module draws the two rigs at the very frame where the claim is worst,
//! from one camera, in the character's own paint, with nothing marked
//! and no rig named: the sides are labelled only LEFT and RIGHT, and
//! which rig is on which side is flipped by a seeded coin. The judge
//! answers "left", "right" or "can't tell" per pair. Two extra pairs
//! keep the judge honest:
//!
//! - a *can't-tell control*: the same rig on both sides. Anything but
//!   "can't tell" means the judge is guessing, and the run fails.
//! - a *planted bad case*: one side is a copy of a rig with a mirrored
//!   limb's weights swapped left/right. It must look worse, and the
//!   tears measure must flag it, or the run fails.
//!
//! The game camera is the game's follow camera, its distance and lens
//! read from the game's own metrics.ron (`game_camera`: 3.0 m and a 75
//! degree view), so a whole 1.75 m body fills about 38% of the frame
//! height. The heading is whichever follow-camera heading shows the
//! claim best (the in-game camera orbits as the player turns), but the
//! distance and framing never close in on the fault: what the judge
//! sees is what the player would see.
//!
//! Scoring (`score_pairs`) lays the judge's answers against the tears
//! measure (kept) and the old 0-100 score. Where the judge and tears
//! agree against the score, that claim's score verdict is *retired*:
//! it is recorded as disagreed-and-dropped, never as evidence.

use crate::math::Vec3;
use crate::metrics::Ctx;
use crate::owner;
use crate::paint::Paint;
use crate::render::{Fit, Image, View, draw_mesh, fit};
use crate::scene::{Model, Weights, normalize_vw};
use crate::skin::lbs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The game's follow camera, read from a `metrics.ron` camera config
/// (`distance` behind the focus point in metres, vertical `fov` in
/// degrees). Supplied via `--metrics <path>` or `METRICS_FILE`
/// (no hardcoded game checkout path fallback).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameCamera {
    pub distance_m: f64,
    pub fov_deg: f64,
}

/// The `camera` block of a metrics.ron text (JSON after its `//` lines).
pub fn parse_game_camera(text: &str) -> Result<GameCamera, String> {
    let body: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("metrics.ron: {e}"))?;
    let cam = v.get("camera").ok_or("metrics.ron has no camera block")?;
    let num = |k: &str| cam.get(k).and_then(|x| x.as_f64()).ok_or(format!("metrics.ron camera has no {k}"));
    let distance_m = num("distance")?;
    let fov_deg = num("fov")?;
    if !distance_m.is_finite() || distance_m <= 0.0 {
        return Err(format!("metrics.ron camera distance {distance_m} must be positive and finite"));
    }
    if !fov_deg.is_finite() || !(fov_deg > 0.0 && fov_deg < 180.0) {
        return Err(format!("metrics.ron camera fov {fov_deg} must be between 0 and 180"));
    }
    Ok(GameCamera { distance_m, fov_deg })
}

/// Explicit metrics.ron path from `METRICS_FILE`, if set.
pub fn game_metrics_path() -> Option<std::path::PathBuf> {
    std::env::var_os("METRICS_FILE").filter(|p| !p.is_empty()).map(Into::into)
}

/// Reads the follow camera from an explicit `metrics.ron` file.
pub fn game_camera_from_path(path: &std::path::Path) -> Result<GameCamera, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("the game's camera: cannot read {} ({e})", path.display()))?;
    parse_game_camera(&text)
}

/// The game's follow camera from `METRICS_FILE`.
pub fn game_camera() -> Result<GameCamera, String> {
    let p = game_metrics_path().ok_or_else(|| "the game's camera: pass --metrics <metrics.ron> (or set METRICS_FILE)".to_string())?;
    game_camera_from_path(&p)
}

/// How much of the frame height a 1.75 m body fills at a camera:
/// `1.75 / (2 * distance * tan(fov / 2))`.
pub fn frame_share(cam: GameCamera) -> f64 {
    owner::BODY_M / (2.0 * cam.distance_m * (cam.fov_deg * 0.5).to_radians().tan())
}

/// How much of the frame height a 1.75 m body fills at an explicit `metrics.ron`.
pub fn game_frame_share_from_path(path: &std::path::Path) -> Result<f64, String> {
    game_camera_from_path(path).map(frame_share)
}

/// How much of the frame height a 1.75 m body fills at the game's follow
/// camera (0.38 at 3.0 m and 75 degrees).
pub fn game_frame_share() -> Result<f64, String> {
    game_camera().map(frame_share)
}

/// The game's talk camera, the close shot (`talk::frame`, `Shot::Close`):
/// the eye about 1.3 m from the speaker's face (three quarters of a
/// 1.73 m talk distance, at most 1.3 m), level with it, aimed 0.1 m under
/// it, through a 38 degree lens, 24 degrees off the line between the two.
pub const TALK_CAM_DISTANCE_M: f64 = 1.3;
pub const TALK_CAM_FOV_DEG: f64 = 38.0;
/// The face's height as a share of the body's (`talk::FACE`).
pub const TALK_FACE_SHARE: f64 = 0.915;
/// The talk camera aims this far under the face, metres (1.75 m body).
pub const TALK_AIM_BELOW_M: f64 = 0.1;
/// How far off the line between the two the close shot sits, degrees.
pub const TALK_YAW_DEG: f64 = 24.0;
/// Level with the face, looking 0.1 m under it from 1.3 m.
pub const TALK_PITCH_DEG: f64 = 4.4;

/// How much of a 1.75 m body the talk camera's frame height shows,
/// metres: `2 * 1.3 * tan(19deg)`, about 0.90.
pub fn talk_visible_m() -> f64 {
    2.0 * TALK_CAM_DISTANCE_M * (TALK_CAM_FOV_DEG * 0.5).to_radians().tan()
}

/// The talk frame's lowest and highest points, as shares of the body's
/// height from the floor (about 0.60 to 1.11: head, shoulders, chest).
pub fn talk_frame_span() -> (f64, f64) {
    let aim = TALK_FACE_SHARE - TALK_AIM_BELOW_M / owner::BODY_M;
    let half = 0.5 * talk_visible_m() / owner::BODY_M;
    (aim - half, aim + half)
}

/// Which camera a blind run draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Camera {
    /// The game follow camera: the whole body, about 38% of the frame.
    Follow,
    /// The game talk camera, the close shot: head, shoulders and chest.
    Talk,
}

impl Camera {
    pub fn parse(s: &str) -> Option<Camera> {
        match s {
            "follow" => Some(Camera::Follow),
            "talk" => Some(Camera::Talk),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Camera::Follow => "follow",
            Camera::Talk => "talk",
        }
    }
}

/// The talk camera's view of a claim: 24 degrees off the front, on the
/// side the claim is on (`side_x`: the claim's x less the body's middle;
/// the character faces +Z, its left is +X).
pub fn talk_view(side_x: f64) -> View {
    View { yaw_deg: if side_x < 0.0 { -TALK_YAW_DEG } else { TALK_YAW_DEG }, pitch_deg: TALK_PITCH_DEG }
}

/// The talk camera's fit: one centre for both rigs, under the face, the
/// scale fixed by the talk camera's distance and lens, so a body
/// `rest_height` tall shows `talk_visible_m()` of it per 1.75 m, whatever
/// the pose does. The face is where the pose has carried it (`posed`: the
/// same vertices as `rest` in this frame), so a frame where the body
/// leans, lunges or falls is still framed on the face, as the game's talk
/// camera frames the speaker where they stand.
pub fn talk_fit(view: &View, rest: &[Vec3], posed: &[Vec3], rest_height: f64, cell_h: usize) -> Fit {
    let floor = rest.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    let k = rest_height.max(1e-9) / owner::BODY_M;
    let head_lo = floor + 0.85 * rest_height;
    let (mut rest_c, mut posed_c, mut n) = (crate::math::v3(0.0, 0.0, 0.0), crate::math::v3(0.0, 0.0, 0.0), 0usize);
    for (r, p) in rest.iter().zip(posed.iter()) {
        if r.y >= head_lo {
            rest_c += *r;
            posed_c += *p;
            n += 1;
        }
    }
    let aim = if n > 0 {
        let (rc, pc) = (rest_c * (1.0 / n as f64), posed_c * (1.0 / n as f64));
        // The aim sits as far under the head's middle as it does at rest.
        let below = rc.y - (floor + TALK_FACE_SHARE * rest_height - TALK_AIM_BELOW_M * k);
        crate::math::v3(pc.x, pc.y - below, pc.z)
    } else {
        crate::math::v3(0.0, floor + TALK_FACE_SHARE * rest_height - TALK_AIM_BELOW_M * k, 0.0)
    };
    let c = view.to_cam(aim);
    Fit { cx: c.x, cy: c.y, ppu: cell_h as f64 / (talk_visible_m() * k) }
}

/// How upright a frame is: the posed head's middle above the floor, as a
/// share of the rest height (about 0.92 standing; a body lying down is
/// near 0.1).
pub fn head_share(rest: &[Vec3], posed: &[Vec3]) -> f64 {
    let floor = rest.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    let top = rest.iter().map(|p| p.y).fold(f64::MIN, f64::max);
    let h = (top - floor).max(1e-9);
    let (mut y, mut n) = (0.0, 0usize);
    for (r, p) in rest.iter().zip(posed.iter()) {
        if r.y >= floor + 0.85 * h {
            y += p.y;
            n += 1;
        }
    }
    if n == 0 { 0.0 } else { (y / n as f64 - floor) / h }
}

/// Is a claim inside the talk frame: the mean rest height of its skin
/// above the frame's lowest point.
pub fn in_talk_frame(rest: &[Vec3], verts: &[u32]) -> bool {
    if verts.is_empty() || rest.is_empty() {
        return false;
    }
    let floor = rest.iter().map(|p| p.y).fold(f64::MAX, f64::min);
    let top = rest.iter().map(|p| p.y).fold(f64::MIN, f64::max);
    let h = (top - floor).max(1e-9);
    let mean = verts.iter().map(|&v| rest[v as usize].y).sum::<f64>() / verts.len() as f64;
    (mean - floor) / h >= talk_frame_span().0
}

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Which side rig A goes on for `pair`, by a seeded coin (true: A is
/// on the left). Deterministic for a given seed, so a judged run can
/// be re-made exactly.
pub fn a_on_left(seed: u64, pair: usize) -> bool {
    let mut s = seed ^ (pair as u64).wrapping_mul(0xD1B5_4A32_D192_ED03);
    splitmix(&mut s) & 1 == 0
}

/// A seeded shuffle of `0..n` (the order pairs are presented in, so
/// the control and the planted case are not in a known place).
pub fn presentation_order(n: usize, seed: u64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    let mut s = seed ^ 0xA076_1D64_78BD_642F;
    for i in (1..n).rev() {
        let j = (splitmix(&mut s) % (i as u64 + 1)) as usize;
        idx.swap(i, j);
    }
    idx
}

/// One rig at one frame, ready to draw blind.
pub struct BlindSide<'a> {
    pub model: &'a Model,
    pub ctx: &'a Ctx,
    pub weights: &'a Weights,
    pub paint: &'a Paint,
}

/// The heading that shows the marked skin best, of the headings a
/// follow camera can sit at (every 30 degrees, three heights).
pub fn claim_view(side: &BlindSide, pose: usize, marked: &[bool]) -> View {
    let posed = lbs(side.model, &side.ctx.mats[pose.min(side.ctx.mats.len().saturating_sub(1))], side.weights);
    crate::render::best_view(&posed, &side.model.tris, marked)
}

/// The fit of the game camera for a pair: one centre for both rigs,
/// the scale fixed by the follow camera's distance and lens, so the
/// body fills `share` (`game_frame_share()`) of the frame height
/// whatever the pose does.
pub fn game_fit(view: &View, posed: &[&[Vec3]], rest_height: f64, cell_w: usize, cell_h: usize, share: f64) -> Fit {
    let base = fit(view, posed, cell_w, cell_h);
    let visible = rest_height.max(1e-9) / share;
    Fit { cx: base.cx, cy: base.cy, ppu: cell_h as f64 / visible }
}

fn blind_colors(model: &Model, paint: &Paint) -> Vec<[f64; 3]> {
    // The character's own paint, dimmed the way the finding sheets
    // dim it, so shape reads first and paint second.
    (0..model.nverts())
        .map(|v| {
            let c = paint.rgb[v];
            let l = 0.3 * c[0] as f64 + 0.59 * c[1] as f64 + 0.11 * c[2] as f64;
            [
                0.15 + 0.6 * (0.7 * c[0] as f64 + 0.3 * l),
                0.15 + 0.6 * (0.7 * c[1] as f64 + 0.3 * l),
                0.15 + 0.6 * (0.7 * c[2] as f64 + 0.3 * l),
            ]
        })
        .collect()
}

/// One side's picture: the whole body at `pose` from `view`, framed
/// by `f`, labelled only with `label` ("LEFT" or "RIGHT").
pub fn cell_image(side: &BlindSide, pose: usize, view: &View, f: Fit, w: usize, h: usize, label: &str) -> Image {
    let mut img = Image::new(w, h, crate::render::BG);
    img.rect(2, 2, w - 4, h - 4, [32, 35, 42]);
    let posed = lbs(side.model, &side.ctx.mats[pose.min(side.ctx.mats.len().saturating_sub(1))], side.weights);
    let colors = blind_colors(side.model, side.paint);
    draw_mesh(&mut img, 2, 2, w - 4, h - 4, &posed, &side.model.tris, &colors, view, f);
    if !label.is_empty() {
        img.text(12, h - 30, label, 3, [232, 234, 238]);
    }
    img
}

/// The pair as one sheet: the two cells side by side under "PAIR NN"
/// and the question. No rig name, no score, nothing marked.
pub fn pair_sheet(left: &Image, right: &Image, pair: usize) -> Image {
    const HEAD: usize = 52;
    let mut img = Image::new(left.w + right.w, left.h + HEAD, crate::render::BG);
    img.text(12, 10, &format!("PAIR {pair:02}"), 3, [232, 234, 238]);
    img.text(12, 36, "WHICH SIDE'S SKIN BENDS AND FOLLOWS CLEANER? ANSWER LEFT, RIGHT, OR CAN'T TELL.", 1, [150, 156, 168]);
    for (src, x0) in [(left, 0usize), (right, left.w)] {
        for y in 0..src.h {
            for x in 0..src.w {
                let o = (y * src.w + x) * 3;
                let d = ((y + HEAD) * img.w + x0 + x) * 3;
                img.rgb[d..d + 3].copy_from_slice(&src.rgb[o..o + 3]);
            }
        }
    }
    img
}

/// The vertices of `dst` that sit where `verts` of `src` sit (nearest
/// rest position), so a claim made on one rig of a body can be
/// measured on another rig of the same body.
pub fn map_vertices(src: &Model, dst: &Model, verts: &[u32]) -> Vec<u32> {
    if src.nverts() == dst.nverts() && src.rest.iter().zip(&dst.rest).all(|(a, b)| (*a - *b).len() < 1e-6) {
        return verts.to_vec();
    }
    verts
        .iter()
        .map(|&v| {
            let p = src.rest[v as usize];
            (0..dst.nverts()).min_by(|&i, &j| (dst.rest[i] - p).len().total_cmp(&(dst.rest[j] - p).len())).unwrap_or(0) as u32
        })
        .collect()
}

/// A deliberately bad copy of a rig: the weights of a mirrored limb
/// pair (left upper arm with right upper arm, and so on) swapped, so
/// each side follows the other side's bone. Returns the weights, the
/// frame where the swap tears most, that tear's vertices, and what
/// was planted. `None` when the skeleton has no mirrored limb pair
/// the swap can tear.
pub fn planted_bad(model: &Model, ctx: &Ctx, motion: bool) -> Option<(Weights, usize, Vec<u32>, String)> {
    let roles = &model.skel.anatomy.roles;
    const FIXED: &[&str] = &["upper_arm", "forearm", "thigh", "shin", "hand", "foot"];
    let mut bases: Vec<String> = FIXED.iter().map(|s| (*s).to_string()).collect();
    for key in roles.keys() {
        if let Some(base) = key.strip_suffix(".L")
            && !FIXED.contains(&base)
            && roles.contains_key(&format!("{base}.R"))
        {
            bases.push(base.to_string());
        }
    }
    let mut best: Option<(Weights, usize, Vec<u32>, String, usize)> = None;
    for base in &bases {
        let (Some(l), Some(r)) = (roles.get(&format!("{base}.L")), roles.get(&format!("{base}.R"))) else { continue };
        let mut swap: BTreeMap<u32, u32> = BTreeMap::new();
        for (a, b) in l.iter().zip(r.iter()) {
            swap.insert(*a as u32, *b as u32);
            swap.insert(*b as u32, *a as u32);
        }
        if swap.is_empty() {
            continue;
        }
        let w: Weights = model
            .weights
            .iter()
            .map(|vw| normalize_vw(vw.iter().map(|&(j, x)| (swap.get(&j).copied().unwrap_or(j), x)).collect()))
            .collect();
        let ev = crate::metrics::evaluate_with(model, ctx, &w, false);
        let rep = crate::report::build("planted", model, ctx, &ev);
        let sec = owner::assess(model, ctx, &ev, &rep, &w, motion, "", Vec::new(), String::new());
        let torn: usize = {
            let mut t = vec![false; model.nverts()];
            for f in sec.findings.iter().filter(|f| f.severity == "fail" && f.kind != "limb_through") {
                for &v in &f.vertices {
                    t[v as usize] = true;
                }
            }
            t.iter().filter(|&&x| x).count()
        };
        if torn == 0 {
            continue;
        }
        let f = sec
            .findings
            .iter()
            .filter(|f| f.severity == "fail" && f.kind != "limb_through")
            .max_by(|a, b| a.area_cm2.total_cmp(&b.area_cm2))?;
        let cand = (w, f.worst_pose, f.vertices.clone(), format!("the left and right {base} weights were swapped"), torn);
        // A swap that tears a real patch is planted enough; do not
        // shop for a worse one at the price of judging every pair.
        if torn >= 25 {
            return Some((cand.0, cand.1, cand.2, cand.3));
        }
        if best.as_ref().is_none_or(|b| cand.4 > b.4) {
            best = Some(cand);
        }
    }
    best.map(|(w, p, v, d, _)| (w, p, v, d))
}

// ---- The answer key and the scoring of a judge's answers ----

/// One pair as the key records it (never in the judged folder).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairKey {
    pub pair: usize,
    /// "claim", "control" (same rig both sides) or "planted".
    pub category: String,
    pub focus: String,
    pub frame: String,
    pub left_rig: String,
    pub right_rig: String,
    /// The answer a sound judge gives: a rig id, or "cant_tell" for
    /// the control. For a claim, the tears measure's winner.
    pub expected: String,
    /// Torn skin on the claim's area per rig, cm^2 (1.75 m body).
    pub tears_cm2: BTreeMap<String, f64>,
    pub tears_winner: String,
    pub score_winner: String,
}

/// A whole blind run's key.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlindKey {
    pub rig_a: String,
    pub rig_b: String,
    pub section: String,
    pub seed: u64,
    /// What was planted for the planted bad case.
    #[serde(default)]
    pub planted: String,
    pub tears_pct: BTreeMap<String, f64>,
    pub scores: BTreeMap<String, f64>,
    pub pairs: Vec<PairKey>,
    /// "follow" or "talk" (older keys: follow).
    #[serde(default)]
    pub camera: String,
    /// Claims left out at the talk camera: their skin is under its frame.
    #[serde(default)]
    pub out_of_frame: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PairVerdict {
    pub pair: usize,
    pub category: String,
    pub focus: String,
    pub answer: Option<String>,
    pub judge_rig: Option<String>,
    pub tears_winner: String,
    pub score_winner: String,
    /// The judge and tears agree against the old score: this claim's
    /// score verdict is retired.
    pub score_retired: bool,
    pub note: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlindVerdict {
    pub pairs: Vec<PairVerdict>,
    pub claims_judged: usize,
    pub judge_agrees_tears: usize,
    pub score_agrees_judge: usize,
    pub score_retired_pairs: Vec<usize>,
    pub control_ok: bool,
    pub planted_ok: bool,
    pub planted_tears_ok: bool,
    /// The run can be believed: the control was answered "can't
    /// tell", the planted bad side was picked out, and tears flag
    /// the planted rig too.
    pub pass: bool,
}

/// Lays a judge's answers against a run's key. `answers` maps a pair
/// number to "left", "right" or "cant_tell" (any spelling of can't:
/// "cant_tell", "can't tell", "cant", "neither").
pub fn score_pairs(key: &BlindKey, answers: &BTreeMap<usize, String>) -> BlindVerdict {
    let norm = |s: &str| -> String {
        let s: String = s.trim().to_lowercase().chars().filter(|c| !matches!(c, '\'' | ' ' | '-' | '_')).collect();
        if ["canttell", "cant", "neither", "same", "equal"].contains(&s.as_str()) { "cant_tell".into() } else { s }
    };
    let mut out = BlindVerdict {
        pairs: Vec::new(),
        claims_judged: 0,
        judge_agrees_tears: 0,
        score_agrees_judge: 0,
        score_retired_pairs: Vec::new(),
        control_ok: false,
        planted_ok: false,
        planted_tears_ok: false,
        pass: false,
    };
    let mut saw_control = false;
    let mut saw_planted = false;
    for p in &key.pairs {
        let answer = answers.get(&p.pair).map(|a| norm(a));
        let judge_rig = match answer.as_deref() {
            Some("left") => Some(p.left_rig.clone()),
            Some("right") => Some(p.right_rig.clone()),
            _ => None,
        };
        let mut retired = false;
        let note: String;
        match p.category.as_str() {
            "control" => {
                saw_control = true;
                out.control_ok = answer.as_deref() == Some("cant_tell");
                note = if out.control_ok {
                    "control: the same rig both sides, answered can't tell".into()
                } else {
                    "control FAILED: the same rig on both sides was not answered can't tell".into()
                };
            }
            "planted" => {
                saw_planted = true;
                out.planted_ok = judge_rig.as_deref() == Some(p.expected.as_str());
                out.planted_tears_ok = p.tears_winner == p.expected;
                note = match (out.planted_ok, out.planted_tears_ok) {
                    (true, true) => "planted bad case: picked out by the judge, flagged by tears".into(),
                    (true, false) => "planted bad case: picked out by the judge but NOT flagged by tears".into(),
                    _ => "planted bad case FAILED: the judge did not pick the unplanted rig".into(),
                };
            }
            _ => {
                if judge_rig.is_some() {
                    out.claims_judged += 1;
                    if judge_rig.as_deref() == Some(p.tears_winner.as_str()) {
                        out.judge_agrees_tears += 1;
                    }
                    if judge_rig.as_deref() == Some(p.score_winner.as_str()) {
                        out.score_agrees_judge += 1;
                    }
                    retired = judge_rig.as_deref() == Some(p.tears_winner.as_str())
                        && p.score_winner != p.tears_winner
                        && judge_rig.as_deref() != Some(p.score_winner.as_str());
                    if retired {
                        note = "judge and tears agree against the 0-100 score: its verdict on this claim is retired".into();
                    } else if judge_rig.as_deref() == Some(p.score_winner.as_str()) && p.score_winner != p.tears_winner {
                        note = "judge backs the score against tears on this claim".into();
                    } else if judge_rig.as_deref() != Some(p.tears_winner.as_str()) {
                        note = "judge disagrees with tears on this claim".into();
                    } else {
                        note = "judge, tears and score agree".into();
                    }
                } else if answer.is_some() {
                    note = "answered can't tell: tears claim not confirmed by eye".into();
                } else {
                    note = "no answer".into();
                }
            }
        }
        if retired {
            out.score_retired_pairs.push(p.pair);
        }
        out.pairs.push(PairVerdict {
            pair: p.pair,
            category: p.category.clone(),
            focus: p.focus.clone(),
            answer,
            judge_rig,
            tears_winner: p.tears_winner.clone(),
            score_winner: p.score_winner.clone(),
            score_retired: retired,
            note,
        });
    }
    out.pass = saw_control && saw_planted && out.control_ok && out.planted_ok && out.planted_tears_ok;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_camera_frames_a_body_like_the_game() {
        // The game's own block: 1.75 m at 3.0 m through a 75 degree view fills ~38%.
        let text = "// header\n{\n \"camera\": {\"distance\": 3.0, \"focus\": 1.535, \"fov\": 75.0}\n}\n";
        let cam = parse_game_camera(text).unwrap();
        assert_eq!(cam, GameCamera { distance_m: 3.0, fov_deg: 75.0 });
        assert!((frame_share(cam) - 0.380).abs() < 0.005, "{}", frame_share(cam));
        assert!(parse_game_camera("{}").is_err());
    }

    #[test]
    fn the_talk_camera_frames_the_head_shoulders_and_chest() {
        // 2 * 1.3 m * tan(19 deg) = 0.895 m of a 1.75 m body.
        assert!((talk_visible_m() - 0.895).abs() < 0.005, "{}", talk_visible_m());
        let (lo, hi) = talk_frame_span();
        assert!((0.55..0.65).contains(&lo) && hi > 1.0, "{lo} {hi}");
        // A 1.75 m column of points: the fit centres under the face and
        // shows the top of the head, not the knees.
        let rest: Vec<Vec3> = (0..=175).map(|i| crate::math::v3(0.0, i as f64 * 0.01, 0.0)).collect();
        let view = talk_view(1.0);
        let f = talk_fit(&view, &rest, &rest, 1.75, 640);
        let screen_y = |y: f64| 320.0 - (view.to_cam(crate::math::v3(0.0, y, 0.0)).y - f.cy) * f.ppu;
        assert!(screen_y(1.75) > 0.0, "the crown is in the frame");
        assert!(screen_y(0.5) > 640.0, "the knees are under it");
        assert!(in_talk_frame(&rest, &[160, 170]) && !in_talk_frame(&rest, &[20, 50]));
        assert_eq!(talk_view(-0.2).yaw_deg, -TALK_YAW_DEG);
        assert_eq!(Camera::parse("talk"), Some(Camera::Talk));
        // The same column lying down: the frame follows the face down to it.
        let lying: Vec<Vec3> = rest.iter().map(|p| crate::math::v3(p.y, 0.1, 0.0)).collect();
        let fl = talk_fit(&view, &rest, &lying, 1.75, 640);
        let head = view.to_cam(crate::math::v3(1.70, 0.1, 0.0));
        let sy = 320.0 - (head.y - fl.cy) * fl.ppu;
        assert!((0.0..640.0).contains(&sy), "the fallen head stays in the frame: {sy}");
        assert!(head_share(&rest, &rest) > 0.85 && head_share(&rest, &lying) < 0.2);
    }

    #[test]
    fn the_coin_and_the_order_are_seeded() {
        let a: Vec<bool> = (1..=8).map(|p| a_on_left(7, p)).collect();
        assert_eq!(a, (1..=8).map(|p| a_on_left(7, p)).collect::<Vec<_>>());
        assert!(a.iter().any(|&x| x) && a.iter().any(|&x| !x), "{a:?}");
        let mut o = presentation_order(6, 3);
        assert_eq!(o, presentation_order(6, 3));
        o.sort_unstable();
        assert_eq!(o, vec![0, 1, 2, 3, 4, 5]);
    }

    fn key_pair(pair: usize, cat: &str, left: &str, right: &str, expected: &str, tears: &str, score: &str) -> PairKey {
        PairKey {
            pair,
            category: cat.into(),
            focus: "f".into(),
            frame: "p".into(),
            left_rig: left.into(),
            right_rig: right.into(),
            expected: expected.into(),
            tears_cm2: BTreeMap::new(),
            tears_winner: tears.into(),
            score_winner: score.into(),
        }
    }

    #[test]
    fn scoring_retires_the_score_only_where_the_judge_and_tears_agree_against_it() {
        let key = BlindKey {
            rig_a: "a.glb".into(),
            rig_b: "b.glb".into(),
            section: "poses".into(),
            seed: 1,
            planted: "test plant".into(),
            tears_pct: BTreeMap::new(),
            scores: BTreeMap::new(),
            camera: "follow".into(),
            out_of_frame: Vec::new(),
            pairs: vec![
                key_pair(1, "claim", "A", "B", "B", "B", "A"), // score says A, tears say B
                key_pair(2, "control", "A", "A", "cant_tell", "tie", "tie"),
                key_pair(3, "planted", "PLANTED_A", "A", "A", "A", "A"),
                key_pair(4, "claim", "A", "B", "A", "A", "A"), // all agree
            ],
        };
        let answers: BTreeMap<usize, String> =
            [(1, "right".to_string()), (2, "can't tell".to_string()), (3, "right".to_string()), (4, "left".to_string())]
                .into_iter()
                .collect();
        let v = score_pairs(&key, &answers);
        assert!(v.pass, "{v:?}");
        assert_eq!(v.score_retired_pairs, vec![1]);
        assert_eq!(v.judge_agrees_tears, 2);
        assert_eq!(v.score_agrees_judge, 1);
        // The judge picking the planted side fails the run.
        let bad: BTreeMap<usize, String> =
            [(1, "right".to_string()), (2, "cant_tell".to_string()), (3, "left".to_string()), (4, "left".to_string())]
                .into_iter()
                .collect();
        assert!(!score_pairs(&key, &bad).pass);
        // So does claiming a difference on the control.
        let bad: BTreeMap<usize, String> =
            [(1, "right".to_string()), (2, "left".to_string()), (3, "right".to_string()), (4, "left".to_string())].into_iter().collect();
        assert!(!score_pairs(&key, &bad).pass);
    }

    #[test]
    fn parse_game_camera_rejects_nonpositive_or_invalid_fov() {
        assert!(parse_game_camera(r#"{"camera": {"distance": 0.0, "fov": 45.0}}"#).is_err());
        assert!(parse_game_camera(r#"{"camera": {"distance": -2.0, "fov": 45.0}}"#).is_err());
        assert!(parse_game_camera(r#"{"camera": {"distance": 3.0, "fov": 0.0}}"#).is_err());
        assert!(parse_game_camera(r#"{"camera": {"distance": 3.0, "fov": 180.0}}"#).is_err());
    }
}
