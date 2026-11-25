//! Creature-gait clips in `weights check --motion` and `blind::planted_bad`
//! (meshweights-5):
//!
//! 1. Quadrupeds checked with `--motion <creature_gait.glb>` (without
//!    `--motion-clips`) play the creature's clips (`walk`, `trot`, `lunge`,
//!    `idle`) instead of skipping them for humanoid `GAME_CLIPS`, report
//!    clip tears in the `"game clips"` headline section, and flag a planted
//!    left/right leg swap on those clips.
//! 2. Multi-legged (`Body::Legged`) creatures transfer clips across rigs,
//!    report clip tears, and flag a planted leg swap on clips.
//! 3. Legless / custom (`Body::Other`) creatures checked with `--motion`
//!    from a clip-bearing copy of the same skeleton (as `creature-clips`
//!    produces) play all clips and report clip tears.

use serde_json::json;
use std::path::PathBuf;
use weightforge_core::anatomy::Body;
use weightforge_core::blind::planted_bad;
use weightforge_core::fixtures::{Bone, Fixture, to_glb};
use weightforge_core::glb::Glb;
use weightforge_core::math::{Quat, Vec3, v3};
use weightforge_core::metrics::CtxOpts;
use weightforge_core::owner;
use weightforge_core::scene::{Model, VW, normalize_vw};
use weightforge_core::{MotionSpec, check_full, check_full_with_gait};

fn f32_bytes(xs: &[f32]) -> Vec<u8> {
    let mut b = Vec::with_capacity(xs.len() * 4);
    for x in xs {
        b.extend_from_slice(&x.to_le_bytes());
    }
    b
}

fn smoothstep(x: f64) -> f64 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[allow(clippy::too_many_arguments)]
fn add_tube(
    pos: &mut Vec<Vec3>,
    tris: &mut Vec<[u32; 3]>,
    w: &mut Vec<VW>,
    pts: &[Vec3],
    up0: Vec3,
    step: f64,
    nseg: usize,
    rx: f64,
    ry: f64,
    chain: &[(u32, f64)],
    band: f64,
) {
    let mut samples: Vec<(Vec3, f64)> = Vec::new();
    let mut s0 = 0.0;
    for seg in pts.windows(2) {
        let l = (seg[1] - seg[0]).len();
        let n = ((l / step).ceil() as usize).max(2);
        for i in 0..n {
            let f = i as f64 / n as f64;
            samples.push((seg[0].lerp(seg[1], f), s0 + l * f));
        }
        s0 += l;
    }
    samples.push((*pts.last().unwrap(), s0));

    let m = samples.len();
    let tangents: Vec<Vec3> = (0..m)
        .map(|i| {
            let a = samples[i.saturating_sub(1)].0;
            let b = samples[(i + 1).min(m - 1)].0;
            (b - a).normalized()
        })
        .collect();
    let mut normal = (up0 - tangents[0] * up0.dot(tangents[0])).normalized();
    let base = pos.len() as u32;

    let weights_at = |s: f64| -> VW {
        let n = chain.len();
        let mut ws = vec![1.0; n];
        for (k, &(_, b)) in chain.iter().enumerate().skip(1) {
            let t = if band <= 0.0 { if s >= b { 1.0 } else { 0.0 } } else { smoothstep((s - (b - band)) / (2.0 * band)) };
            for (i, wi) in ws.iter_mut().enumerate() {
                *wi *= if i < k { 1.0 - t } else { t };
            }
        }
        normalize_vw(chain.iter().zip(ws).map(|(&(j, _), x)| (j, x)).collect())
    };

    for i in 0..m {
        if i > 0 {
            let q = Quat::from_to(tangents[i - 1], tangents[i]);
            normal = q.rotate(normal);
            normal = (normal - tangents[i] * normal.dot(tangents[i])).normalized();
        }
        let bin = tangents[i].cross(normal);
        let vw = weights_at(samples[i].1);
        for k in 0..nseg {
            let a = std::f64::consts::TAU * k as f64 / nseg as f64;
            pos.push(samples[i].0 + normal * (a.cos() * rx) + bin * (a.sin() * ry));
            w.push(vw.clone());
        }
    }
    for i in 0..m - 1 {
        for k in 0..nseg {
            let a = base + (i * nseg + k) as u32;
            let b = base + (i * nseg + (k + 1) % nseg) as u32;
            let c = a + nseg as u32;
            let d = b + nseg as u32;
            tris.push([a, b, d]);
            tris.push([a, d, c]);
        }
    }
    for (ring, s, flip) in [(0usize, 0.0, true), (m - 1, s0, false)] {
        let center = pos.len() as u32;
        pos.push(samples[ring].0);
        w.push(weights_at(s));
        for k in 0..nseg {
            let a = base + (ring * nseg + k) as u32;
            let b = base + (ring * nseg + (k + 1) % nseg) as u32;
            tris.push(if flip { [center, b, a] } else { [center, a, b] });
        }
    }
}

type ClipSpec<'a> = (&'a str, f64, &'a [(usize, Vec3, f64, f64)]);

/// Appends glTF rotation clips to a fixture GLB. Each clip is
/// `(name, duration_s, &[(bone_index, local_axis, peak_deg, phase_rad)])`.
fn attach_clips(glb: &mut Glb, clips: &[ClipSpec<'_>]) {
    let nodes = glb.json["nodes"].as_array().expect("nodes").clone();
    let mut anims = Vec::new();
    for &(name, dur, tracks) in clips {
        let nkeys = 9usize;
        let times: Vec<f32> = (0..nkeys).map(|i| (dur * i as f64 / (nkeys - 1) as f64) as f32).collect();
        let tv = glb.append_view(&f32_bytes(&times), None).unwrap();
        let ta = glb
            .push_accessor(json!({
                "bufferView": tv,
                "componentType": 5126,
                "count": nkeys,
                "type": "SCALAR",
                "min": [times[0]],
                "max": [times[nkeys - 1]],
            }))
            .unwrap();
        let mut samplers = Vec::new();
        let mut channels = Vec::new();
        for &(node, axis, peak_deg, phase) in tracks {
            let r = nodes[node]["rotation"].as_array().expect("node rotation");
            let rest_q =
                Quat { x: r[0].as_f64().unwrap(), y: r[1].as_f64().unwrap(), z: r[2].as_f64().unwrap(), w: r[3].as_f64().unwrap() }
                    .normalized();
            let mut vals = Vec::with_capacity(nkeys * 4);
            for i in 0..nkeys {
                let u = i as f64 / (nkeys - 1) as f64;
                let ang = peak_deg.to_radians() * (std::f64::consts::TAU * u + phase).sin();
                let q = (rest_q * Quat::from_axis_angle(axis.normalized(), ang)).normalized();
                vals.extend_from_slice(&[q.x as f32, q.y as f32, q.z as f32, q.w as f32]);
            }
            let vv = glb.append_view(&f32_bytes(&vals), None).unwrap();
            let va = glb
                .push_accessor(json!({
                    "bufferView": vv,
                    "componentType": 5126,
                    "count": nkeys,
                    "type": "VEC4",
                }))
                .unwrap();
            let si = samplers.len();
            samplers.push(json!({"input": ta, "interpolation": "LINEAR", "output": va}));
            channels.push(json!({"sampler": si, "target": {"node": node, "path": "rotation"}}));
        }
        anims.push(json!({"name": name, "channels": channels, "samplers": samplers}));
    }
    glb.json["animations"] = json!(anims);
    glb.sync_buffer_len().unwrap();
}

/// Procedural clean quadruped facing +Z (`Body::Quadruped`).
/// Bone indices:
/// 0: hips, 1: spine, 2: chest, 3: neck, 4: head, 5: tail,
/// 6..9: back_L (thigh, shin, foot), 9..12: back_R,
/// 12..15: front_L (upper, lower, foot), 15..18: front_R.
fn quadruped_fixture(prefix: &'static str, swap_front_legs: bool) -> Fixture {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let bones = vec![
        Bone { name: leak(format!("{prefix}hips")), parent: None, head: v3(0.0, 0.72, -0.36), tail: v3(0.0, 0.72, -0.05) },
        Bone { name: leak(format!("{prefix}spine")), parent: Some(0), head: v3(0.0, 0.72, -0.05), tail: v3(0.0, 0.74, 0.28) },
        Bone { name: leak(format!("{prefix}chest")), parent: Some(1), head: v3(0.0, 0.74, 0.28), tail: v3(0.0, 0.76, 0.48) },
        Bone { name: leak(format!("{prefix}neck")), parent: Some(2), head: v3(0.0, 0.76, 0.48), tail: v3(0.0, 0.96, 0.68) },
        Bone { name: leak(format!("{prefix}head")), parent: Some(3), head: v3(0.0, 0.96, 0.68), tail: v3(0.0, 0.95, 0.92) },
        Bone { name: leak(format!("{prefix}tail")), parent: Some(0), head: v3(0.0, 0.72, -0.36), tail: v3(0.0, 0.66, -0.78) },
        // Back legs (parent: hips = 0)
        Bone { name: leak(format!("{prefix}back_upper_l")), parent: Some(0), head: v3(0.20, 0.70, -0.34), tail: v3(0.20, 0.36, -0.36) },
        Bone { name: leak(format!("{prefix}back_lower_l")), parent: Some(6), head: v3(0.20, 0.36, -0.36), tail: v3(0.20, 0.06, -0.34) },
        Bone { name: leak(format!("{prefix}back_foot_l")), parent: Some(7), head: v3(0.20, 0.06, -0.34), tail: v3(0.20, 0.02, -0.22) },
        Bone { name: leak(format!("{prefix}back_upper_r")), parent: Some(0), head: v3(-0.20, 0.70, -0.34), tail: v3(-0.20, 0.36, -0.36) },
        Bone { name: leak(format!("{prefix}back_lower_r")), parent: Some(9), head: v3(-0.20, 0.36, -0.36), tail: v3(-0.20, 0.06, -0.34) },
        Bone { name: leak(format!("{prefix}back_foot_r")), parent: Some(10), head: v3(-0.20, 0.06, -0.34), tail: v3(-0.20, 0.02, -0.22) },
        // Front legs (parent: chest = 2)
        Bone { name: leak(format!("{prefix}front_upper_l")), parent: Some(2), head: v3(0.20, 0.72, 0.36), tail: v3(0.20, 0.38, 0.36) },
        Bone { name: leak(format!("{prefix}front_lower_l")), parent: Some(12), head: v3(0.20, 0.38, 0.36), tail: v3(0.20, 0.06, 0.36) },
        Bone { name: leak(format!("{prefix}front_foot_l")), parent: Some(13), head: v3(0.20, 0.06, 0.36), tail: v3(0.20, 0.02, 0.48) },
        Bone { name: leak(format!("{prefix}front_upper_r")), parent: Some(2), head: v3(-0.20, 0.72, 0.36), tail: v3(-0.20, 0.38, 0.36) },
        Bone { name: leak(format!("{prefix}front_lower_r")), parent: Some(15), head: v3(-0.20, 0.38, 0.36), tail: v3(-0.20, 0.06, 0.36) },
        Bone { name: leak(format!("{prefix}front_foot_r")), parent: Some(16), head: v3(-0.20, 0.06, 0.36), tail: v3(-0.20, 0.02, 0.48) },
    ];

    let mut pos = Vec::new();
    let mut tris = Vec::new();
    let mut w = Vec::new();

    // Torso
    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.72, -0.44), v3(0.0, 0.72, -0.05), v3(0.0, 0.74, 0.28), v3(0.0, 0.76, 0.50)],
        v3(1.0, 0.0, 0.0),
        0.04,
        16,
        0.18,
        0.16,
        &[(0, 0.0), (1, 0.39), (2, 0.72)],
        0.06,
    );
    // Neck + head
    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.76, 0.44), v3(0.0, 0.96, 0.68), v3(0.0, 0.95, 0.94)],
        v3(1.0, 0.0, 0.0),
        0.04,
        14,
        0.09,
        0.09,
        &[(3, 0.0), (4, 0.31)],
        0.05,
    );
    // Tail
    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.72, -0.36), v3(0.0, 0.66, -0.80)],
        v3(1.0, 0.0, 0.0),
        0.04,
        10,
        0.04,
        0.04,
        &[(5, 0.0)],
        0.03,
    );
    // Four legs
    let leg_specs: [([Vec3; 4], [u32; 3], [u32; 3]); 4] = [
        ([v3(0.20, 0.72, -0.34), v3(0.20, 0.36, -0.36), v3(0.20, 0.06, -0.34), v3(0.20, 0.02, -0.22)], [6, 7, 8], [6, 7, 8]),
        ([v3(-0.20, 0.72, -0.34), v3(-0.20, 0.36, -0.36), v3(-0.20, 0.06, -0.34), v3(-0.20, 0.02, -0.22)], [9, 10, 11], [9, 10, 11]),
        ([v3(0.20, 0.74, 0.36), v3(0.20, 0.38, 0.36), v3(0.20, 0.06, 0.36), v3(0.20, 0.02, 0.48)], [12, 13, 14], [15, 16, 17]),
        ([v3(-0.20, 0.74, 0.36), v3(-0.20, 0.38, 0.36), v3(-0.20, 0.06, 0.36), v3(-0.20, 0.02, 0.48)], [15, 16, 17], [12, 13, 14]),
    ];
    for (pts, clean_b, swapped_b) in leg_specs {
        let bs = if swap_front_legs { swapped_b } else { clean_b };
        let l1 = (pts[1] - pts[0]).len();
        let l2 = l1 + (pts[2] - pts[1]).len();
        add_tube(
            &mut pos,
            &mut tris,
            &mut w,
            &pts,
            v3(0.0, 0.0, 1.0),
            0.035,
            12,
            0.055,
            0.055,
            &[(bs[0], 0.0), (bs[1], l1), (bs[2], l2)],
            0.05,
        );
    }

    Fixture { bones, body_pos: pos, body_tris: tris, cape_pos: Vec::new(), cape_tris: Vec::new(), body_w: w, cape_w: Vec::new() }
}

/// Procedural clean 6-legged creature (`Body::Legged`).
/// 0: root, 1: mid, 2: front, 3..15: 6 legs (2 bones each: upper, lower).
fn hexapod_fixture(prefix: &'static str) -> Fixture {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let mut bones = vec![
        Bone { name: leak(format!("{prefix}body_back")), parent: None, head: v3(0.0, 0.50, -0.45), tail: v3(0.0, 0.50, 0.0) },
        Bone { name: leak(format!("{prefix}body_mid")), parent: Some(0), head: v3(0.0, 0.50, 0.0), tail: v3(0.0, 0.50, 0.45) },
        Bone { name: leak(format!("{prefix}body_front")), parent: Some(1), head: v3(0.0, 0.50, 0.45), tail: v3(0.0, 0.50, 0.70) },
    ];
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    let mut w = Vec::new();

    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.50, -0.50), v3(0.0, 0.50, 0.0), v3(0.0, 0.50, 0.45), v3(0.0, 0.50, 0.72)],
        v3(1.0, 0.0, 0.0),
        0.04,
        14,
        0.16,
        0.12,
        &[(0, 0.0), (1, 0.50), (2, 0.95)],
        0.06,
    );

    let pairs: [(usize, f64, &str); 3] = [(2, 0.36, "f"), (1, 0.0, "m"), (0, -0.36, "b")];
    for (pidx, z, tag) in pairs {
        for (side, sfx) in [(1.0, "l"), (-1.0, "r")] {
            let u = bones.len() as u32;
            let p0 = v3(side * 0.16, 0.48, z);
            let p1 = v3(side * 0.34, 0.32, z);
            let p2 = v3(side * 0.36, 0.03, z);
            let p3 = v3(side * 0.38, 0.01, z + 0.08);
            bones.push(Bone { name: leak(format!("{prefix}leg_{tag}_u_{sfx}")), parent: Some(pidx), head: p0, tail: p1 });
            bones.push(Bone { name: leak(format!("{prefix}leg_{tag}_l_{sfx}")), parent: Some(u as usize), head: p1, tail: p2 });
            bones.push(Bone { name: leak(format!("{prefix}leg_{tag}_t_{sfx}")), parent: Some(u as usize + 1), head: p2, tail: p3 });
            let l1 = (p1 - p0).len();
            let l2 = l1 + (p2 - p1).len();
            add_tube(
                &mut pos,
                &mut tris,
                &mut w,
                &[p0, p1, p2, p3],
                v3(0.0, 0.0, 1.0),
                0.03,
                10,
                0.04,
                0.04,
                &[(u, 0.0), (u + 1, l1), (u + 2, l2)],
                0.04,
            );
        }
    }

    Fixture { bones, body_pos: pos, body_tris: tris, cape_pos: Vec::new(), cape_tris: Vec::new(), body_w: w, cape_w: Vec::new() }
}

/// Procedural legless creature (`Body::Other`, e.g. serpent/worm).
fn serpent_fixture() -> Fixture {
    let bones = vec![
        Bone { name: "seg_0", parent: None, head: v3(0.0, 0.25, -0.60), tail: v3(0.0, 0.25, -0.20) },
        Bone { name: "seg_1", parent: Some(0), head: v3(0.0, 0.25, -0.20), tail: v3(0.0, 0.25, 0.20) },
        Bone { name: "seg_2", parent: Some(1), head: v3(0.0, 0.25, 0.20), tail: v3(0.0, 0.25, 0.60) },
        Bone { name: "seg_3", parent: Some(2), head: v3(0.0, 0.25, 0.60), tail: v3(0.0, 0.30, 0.95) },
    ];
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    let mut w = Vec::new();
    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.25, -0.65), v3(0.0, 0.25, -0.20), v3(0.0, 0.25, 0.20), v3(0.0, 0.25, 0.60), v3(0.0, 0.30, 0.98)],
        v3(1.0, 0.0, 0.0),
        0.04,
        14,
        0.09,
        0.09,
        &[(0, 0.0), (1, 0.45), (2, 0.85), (3, 1.25)],
        0.06,
    );
    Fixture { bones, body_pos: pos, body_tris: tris, cape_pos: Vec::new(), cape_tris: Vec::new(), body_w: w, cape_w: Vec::new() }
}

fn temp_path(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wf_creature_clips_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(tag)
}

#[test]
fn quadruped_check_with_gait_motion_plays_all_clips_and_flags_planted_swap() {
    let clean_f = quadruped_fixture("tgt_", false);
    let swapped_f = quadruped_fixture("tgt_", true);
    let src_f = quadruped_fixture("gait_", false);

    let clean_glb = to_glb(&clean_f);
    let swapped_glb = to_glb(&swapped_f);
    let mut gait_glb = to_glb(&src_f);
    // Creature-gait clips made by the creature route: walk, trot, lunge, idle.
    // Front/back legs swing in opposite phase on the left and right sides so
    // any left/right leg weight swap produces a clear tear on clips.
    let pi = std::f64::consts::PI;
    attach_clips(
        &mut gait_glb,
        &[
            (
                "walk",
                0.8,
                &[
                    (6, v3(1.0, 0.0, 0.0), 28.0, 0.0),
                    (9, v3(1.0, 0.0, 0.0), 28.0, pi),
                    (12, v3(1.0, 0.0, 0.0), 28.0, pi),
                    (15, v3(1.0, 0.0, 0.0), 28.0, 0.0),
                ],
            ),
            (
                "trot",
                0.6,
                &[
                    (6, v3(1.0, 0.0, 0.0), 38.0, 0.0),
                    (9, v3(1.0, 0.0, 0.0), 38.0, pi),
                    (12, v3(1.0, 0.0, 0.0), 38.0, pi),
                    (15, v3(1.0, 0.0, 0.0), 38.0, 0.0),
                ],
            ),
            (
                "lunge",
                0.5,
                &[
                    (1, v3(1.0, 0.0, 0.0), 14.0, 0.0),
                    (12, v3(1.0, 0.0, 0.0), 42.0, 0.0),
                    (15, v3(1.0, 0.0, 0.0), 35.0, pi),
                    (3, v3(1.0, 0.0, 0.0), 15.0, 0.0),
                ],
            ),
            ("idle", 1.0, &[(3, v3(1.0, 0.0, 0.0), 6.0, 0.0), (5, v3(0.0, 0.0, 1.0), 12.0, 0.0)]),
        ],
    );

    let clean_path = temp_path("quad_clean.glb");
    let swapped_path = temp_path("quad_swapped.glb");
    let gait_path = temp_path("quad_gait.glb");
    std::fs::write(&clean_path, clean_glb.to_bytes()).unwrap();
    std::fs::write(&swapped_path, swapped_glb.to_bytes()).unwrap();
    std::fs::write(&gait_path, gait_glb.to_bytes()).unwrap();

    // Sanity: both classify as Quadruped despite distinct bone names.
    let clean_m = Model::load(&clean_path).unwrap();
    let gait_m = Model::load(&gait_path).unwrap();
    assert_eq!(clean_m.skel.anatomy.body, Body::Quadruped, "notes: {:?}", clean_m.skel.anatomy.notes);
    assert_eq!(gait_m.skel.anatomy.body, Body::Quadruped, "notes: {:?}", gait_m.skel.anatomy.notes);

    // Check without --motion-clips: all four creature clips must play.
    let spec = MotionSpec { source: Some(gait_path.clone()), clips: None };
    let checked = check_full(&clean_path, &CtxOpts::default(), &spec).unwrap();
    let clip_sec =
        checked.rep.headline.iter().find(|s| s.name == "game clips").expect("quadruped check must produce a 'game clips' headline section");
    assert_eq!(
        clip_sec.clips,
        vec!["walk", "trot", "lunge", "idle"],
        "all creature-gait clips must play by default; anatomy notes: {:?}",
        checked.rep.anatomy
    );
    assert!(clip_sec.frames >= 20, "expected sampled clip frames, got {}", clip_sec.frames);
    assert!(!checked.rep.anatomy.iter().any(|n| n.contains("clip not played")), "unexpected skipped clips: {:?}", checked.rep.anatomy);
    assert_eq!(clip_sec.tears_pct, 0.0, "clean quadruped should have 0% clip tears, findings: {:?}", clip_sec.findings);

    // Planted left/right swap must be found and flagged on the creature clips.
    let (mctx, _, _) = checked.motion.as_ref().expect("motion ctx present");
    let (planted_w, _pose, verts, what) =
        planted_bad(&checked.model, mctx, true).expect("planted_bad must find a mirrored leg swap on quadruped clips");
    assert!(what.contains("swapped"), "{what}");
    assert!(!verts.is_empty());
    let clean_torn = owner::torn(&checked.model, mctx, &checked.model.weights, true);
    let planted_torn = owner::torn(&checked.model, mctx, &planted_w, true);
    assert_eq!(clean_torn.iter().filter(|&&t| t).count(), 0);
    assert!(
        planted_torn.iter().filter(|&&t| t).count() >= 25,
        "planted swap on quadruped clips tore only {} verts",
        planted_torn.iter().filter(|&&t| t).count()
    );

    // Explicitly swapped quadruped must report positive clip tears in check_full.
    let bad = check_full(&swapped_path, &CtxOpts::default(), &spec).unwrap();
    let bad_sec = bad.rep.headline.iter().find(|s| s.name == "game clips").expect("swapped quadruped must produce a 'game clips' section");
    assert!(bad_sec.tears_pct > 0.0, "swapped front legs must report clip tears, got 0.0 (findings: {:?})", bad_sec.findings);
}

#[test]
fn legged_creature_clips_transfer_and_flag_planted_swap() {
    let tgt_glb = to_glb(&hexapod_fixture("hex_a_"));
    let mut src_glb = to_glb(&hexapod_fixture("hex_b_"));
    let pi = std::f64::consts::PI;
    // Bones 3,6,9,12,15,18 are the six upper leg bones.
    attach_clips(
        &mut src_glb,
        &[
            (
                "walk",
                0.7,
                &[
                    (3, v3(0.0, 0.0, 1.0), 30.0, 0.0),
                    (6, v3(0.0, 0.0, 1.0), 30.0, pi),
                    (9, v3(0.0, 0.0, 1.0), 30.0, pi),
                    (12, v3(0.0, 0.0, 1.0), 30.0, 0.0),
                    (15, v3(0.0, 0.0, 1.0), 30.0, 0.0),
                    (18, v3(0.0, 0.0, 1.0), 30.0, pi),
                ],
            ),
            ("attack", 0.5, &[(3, v3(0.0, 0.0, 1.0), 40.0, 0.0), (6, v3(0.0, 0.0, 1.0), 35.0, pi)]),
            ("idle", 0.8, &[(2, v3(1.0, 0.0, 0.0), 8.0, 0.0)]),
        ],
    );

    let tgt_path = temp_path("hex_tgt.glb");
    let src_path = temp_path("hex_clips.glb");
    std::fs::write(&tgt_path, tgt_glb.to_bytes()).unwrap();
    std::fs::write(&src_path, src_glb.to_bytes()).unwrap();

    let m = Model::load(&tgt_path).unwrap();
    assert_eq!(m.skel.anatomy.body, Body::Legged, "notes: {:?}", m.skel.anatomy.notes);

    let spec = MotionSpec { source: Some(src_path), clips: None };
    let checked = check_full(&tgt_path, &CtxOpts::default(), &spec).unwrap();
    let clip_sec =
        checked.rep.headline.iter().find(|s| s.name == "game clips").expect("legged creature must produce a 'game clips' section");
    assert_eq!(clip_sec.clips, vec!["walk", "attack", "idle"]);
    assert!(clip_sec.frames >= 15);
    assert!(!checked.rep.anatomy.iter().any(|n| n.contains("clip not played")));

    let (mctx, _, _) = checked.motion.as_ref().expect("motion ctx present");
    let (planted_w, _pose, verts, what) =
        planted_bad(&checked.model, mctx, true).expect("planted_bad must find a mirrored leg swap on Body::Legged clips");
    assert!(what.contains("swapped"), "{what}");
    assert!(!verts.is_empty());
    let planted_torn = owner::torn(&checked.model, mctx, &planted_w, true);
    assert!(
        planted_torn.iter().filter(|&&t| t).count() >= 20,
        "planted swap on legged creature clips tore only {} verts",
        planted_torn.iter().filter(|&&t| t).count()
    );
}

#[test]
fn same_skeleton_other_creature_plays_clips_from_motion_source() {
    let base_glb = to_glb(&serpent_fixture());
    let mut moving_glb = to_glb(&serpent_fixture());
    attach_clips(
        &mut moving_glb,
        &[
            ("idle", 0.8, &[(1, v3(0.0, 1.0, 0.0), 15.0, 0.0), (2, v3(0.0, 1.0, 0.0), 20.0, 1.2)]),
            ("attack", 0.5, &[(2, v3(1.0, 0.0, 0.0), 30.0, 0.0), (3, v3(1.0, 0.0, 0.0), 35.0, 0.0)]),
        ],
    );

    let base_path = temp_path("serpent_base.glb");
    let moving_path = temp_path("serpent_moving.glb");
    std::fs::write(&base_path, base_glb.to_bytes()).unwrap();
    std::fs::write(&moving_path, moving_glb.to_bytes()).unwrap();

    let m = Model::load(&base_path).unwrap();
    assert_eq!(m.skel.anatomy.body, Body::Other, "notes: {:?}", m.skel.anatomy.notes);

    let spec = MotionSpec { source: Some(moving_path), clips: None };
    let checked = check_full(&base_path, &CtxOpts::default(), &spec).unwrap();
    let clip_sec = checked
        .rep
        .headline
        .iter()
        .find(|s| s.name == "game clips")
        .expect("same-skeleton Body::Other creature must play clips via --motion");
    assert_eq!(clip_sec.clips, vec!["idle", "attack"]);
    assert!(clip_sec.frames >= 10);
    assert!(!checked.rep.anatomy.iter().any(|n| n.contains("clip not played")));
}

#[test]
fn quadruped_with_low_drooping_tail_and_heavy_neck_preserves_hind_legs_and_neck_head() {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let bones = vec![
        Bone { name: leak("DEF-hips".into()), parent: None, head: v3(0.0, 0.72, -0.36), tail: v3(0.0, 0.72, -0.05) },
        Bone { name: leak("DEF-spine.001".into()), parent: Some(0), head: v3(0.0, 0.72, -0.05), tail: v3(0.0, 0.73, 0.25) },
        Bone { name: leak("DEF-spine.002".into()), parent: Some(1), head: v3(0.0, 0.73, 0.25), tail: v3(0.0, 0.73, 0.42) },
        Bone { name: leak("DEF-spine.003".into()), parent: Some(2), head: v3(0.0, 0.73, 0.42), tail: v3(0.0, 0.73, 0.56) },
        Bone { name: leak("DEF-neck".into()), parent: Some(3), head: v3(0.0, 0.73, 0.56), tail: v3(0.0, 0.90, 0.76) },
        Bone { name: leak("DEF-head".into()), parent: Some(4), head: v3(0.0, 0.90, 0.76), tail: v3(0.0, 0.90, 0.96) },
        // Midline tail drooping close to the floor (< 0.12*h) carrying heavier vertex mass than a hind leg.
        Bone { name: leak("DEF-tail.001".into()), parent: Some(0), head: v3(0.0, 0.70, -0.38), tail: v3(0.0, 0.42, -0.54) },
        Bone { name: leak("DEF-tail.002".into()), parent: Some(6), head: v3(0.0, 0.42, -0.54), tail: v3(0.0, 0.11, -0.60) },
        Bone { name: leak("DEF-tail.003".into()), parent: Some(7), head: v3(0.0, 0.11, -0.60), tail: v3(0.0, 0.08, -0.65) },
        // Hind legs (0.20 lateral offset)
        Bone { name: leak("DEF-leg_hind.001.L".into()), parent: Some(0), head: v3(0.20, 0.70, -0.34), tail: v3(0.20, 0.36, -0.36) },
        Bone { name: leak("DEF-leg_hind.002.L".into()), parent: Some(9), head: v3(0.20, 0.36, -0.36), tail: v3(0.20, 0.06, -0.34) },
        Bone { name: leak("DEF-leg_hind.003.L".into()), parent: Some(10), head: v3(0.20, 0.06, -0.34), tail: v3(0.20, 0.02, -0.22) },
        Bone { name: leak("DEF-leg_hind.001.R".into()), parent: Some(0), head: v3(-0.20, 0.70, -0.34), tail: v3(-0.20, 0.36, -0.36) },
        Bone { name: leak("DEF-leg_hind.002.R".into()), parent: Some(12), head: v3(-0.20, 0.36, -0.36), tail: v3(-0.20, 0.06, -0.34) },
        Bone { name: leak("DEF-leg_hind.003.R".into()), parent: Some(13), head: v3(-0.20, 0.06, -0.34), tail: v3(-0.20, 0.02, -0.22) },
        // Front legs off spine.002 (so spine.003 sits inside shoulder span as anterior ribcage)
        Bone { name: leak("DEF-leg_front.001.L".into()), parent: Some(2), head: v3(0.20, 0.72, 0.40), tail: v3(0.20, 0.38, 0.40) },
        Bone { name: leak("DEF-leg_front.002.L".into()), parent: Some(15), head: v3(0.20, 0.38, 0.40), tail: v3(0.20, 0.06, 0.40) },
        Bone { name: leak("DEF-leg_front.003.L".into()), parent: Some(16), head: v3(0.20, 0.06, 0.40), tail: v3(0.20, 0.02, 0.52) },
        Bone { name: leak("DEF-leg_front.001.R".into()), parent: Some(2), head: v3(-0.20, 0.72, 0.40), tail: v3(-0.20, 0.38, 0.40) },
        Bone { name: leak("DEF-leg_front.002.R".into()), parent: Some(18), head: v3(-0.20, 0.38, 0.40), tail: v3(-0.20, 0.06, 0.40) },
        Bone { name: leak("DEF-leg_front.003.R".into()), parent: Some(19), head: v3(-0.20, 0.06, 0.40), tail: v3(-0.20, 0.02, 0.52) },
    ];
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    let mut w = Vec::new();
    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.72, -0.40), v3(0.0, 0.72, -0.05), v3(0.0, 0.73, 0.25), v3(0.0, 0.73, 0.56)],
        v3(1.0, 0.0, 0.0),
        0.04,
        14,
        0.16,
        0.15,
        &[(0, 0.0), (1, 0.35), (2, 0.65), (3, 0.82)],
        0.05,
    );
    // Dense neck ruff + lighter head
    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.73, 0.56), v3(0.0, 0.90, 0.76), v3(0.0, 0.90, 0.98)],
        v3(1.0, 0.0, 0.0),
        0.02,
        18,
        0.12,
        0.12,
        &[(4, 0.0), (5, 0.36)],
        0.04,
    );
    // Dense bushy tail (heavier vertex count than hind legs)
    add_tube(
        &mut pos,
        &mut tris,
        &mut w,
        &[v3(0.0, 0.70, -0.38), v3(0.0, 0.42, -0.54), v3(0.0, 0.11, -0.60), v3(0.0, 0.08, -0.65)],
        v3(1.0, 0.0, 0.0),
        0.015,
        18,
        0.08,
        0.08,
        &[(6, 0.0), (7, 0.30), (8, 0.62)],
        0.04,
    );
    for (pts, j0, j1, j2) in [
        ([v3(0.20, 0.70, -0.34), v3(0.20, 0.36, -0.36), v3(0.20, 0.06, -0.34), v3(0.20, 0.02, -0.22)], 9u32, 10u32, 11u32),
        ([v3(-0.20, 0.70, -0.34), v3(-0.20, 0.36, -0.36), v3(-0.20, 0.06, -0.34), v3(-0.20, 0.02, -0.22)], 12, 13, 14),
        ([v3(0.20, 0.72, 0.40), v3(0.20, 0.38, 0.40), v3(0.20, 0.06, 0.40), v3(0.20, 0.02, 0.52)], 15, 16, 17),
        ([v3(-0.20, 0.72, 0.40), v3(-0.20, 0.38, 0.40), v3(-0.20, 0.06, 0.40), v3(-0.20, 0.02, 0.52)], 18, 19, 20),
    ] {
        add_tube(&mut pos, &mut tris, &mut w, &pts, v3(1.0, 0.0, 0.0), 0.05, 8, 0.05, 0.05, &[(j0, 0.0), (j1, 0.34), (j2, 0.64)], 0.04);
    }
    let m = Model::from_glb(to_glb(&Fixture {
        bones,
        body_pos: pos,
        body_tris: tris,
        body_w: w,
        cape_pos: Vec::new(),
        cape_tris: Vec::new(),
        cape_w: Vec::new(),
    }))
    .unwrap();
    let a = &m.skel.anatomy;
    assert_eq!(a.body, Body::Quadruped, "notes: {:?}", a.notes);
    assert_eq!(a.roles.get("neck").unwrap().iter().map(|&j| m.skel.names[j].as_str()).collect::<Vec<_>>(), ["DEF-neck"]);
    assert_eq!(a.roles.get("head").unwrap().iter().map(|&j| m.skel.names[j].as_str()).collect::<Vec<_>>(), ["DEF-head"]);
    assert!(a.roles.contains_key("thigh.L") && a.roles.contains_key("thigh.R"), "roles: {:?}", a.roles.keys());
    assert!(a.roles.contains_key("tail*"), "roles: {:?}", a.roles.keys());
    assert_eq!(a.group[m.skel.joint_by_name("DEF-tail.001").unwrap()], weightforge_core::anatomy::Group::Tail);
    assert_eq!(a.group[m.skel.joint_by_name("DEF-spine.003").unwrap()], weightforge_core::anatomy::Group::Torso);
}

#[test]
fn deep_55_to_107_bone_creatures_keep_the_full_45_degree_generic_rom() {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    // Build a 64-bone winged + horned + jawed + deep-tailed creature (Body::Legged):
    // spine (4), neck (4), head (1), jaw (1), 2 horns x 3, 2 eyes (2),
    // tail (10), 2 legs x 3 (6), 2 wings x (4 arm + 3*4=12 feathers) = 62 joints.
    let mut bones = Vec::new();
    let mut pos = Vec::new();
    let mut tris = Vec::new();
    let mut w = Vec::new();
    let mut add_chain = |pts: &[Vec3], names: Vec<String>, parent: Option<usize>, rx: f64, ry: f64| -> Vec<usize> {
        let mut ids = Vec::new();
        let mut p = parent;
        for i in 0..pts.len() - 1 {
            let id = bones.len();
            bones.push(Bone { name: leak(names[i].clone()), parent: p, head: pts[i], tail: pts[i + 1] });
            ids.push(id);
            p = Some(id);
        }
        let mut chain_w = Vec::new();
        let mut acc = 0.0;
        for (idx, &id) in ids.iter().enumerate() {
            chain_w.push((id as u32, acc));
            acc += (pts[idx + 1] - pts[idx]).len();
        }
        add_tube(&mut pos, &mut tris, &mut w, pts, v3(1.0, 0.0, 0.0), 0.04, 10, rx, ry, &chain_w, 0.03);
        ids
    };

    let spine_pts = [v3(0.0, 0.60, -0.20), v3(0.0, 0.61, -0.05), v3(0.0, 0.62, 0.10), v3(0.0, 0.63, 0.25), v3(0.0, 0.64, 0.40)];
    let spine = add_chain(&spine_pts, (0..4).map(|i| format!("spine_{i}")).collect(), None, 0.14, 0.12);
    let neck_pts = [v3(0.0, 0.64, 0.40), v3(0.0, 0.72, 0.48), v3(0.0, 0.80, 0.56), v3(0.0, 0.88, 0.64), v3(0.0, 0.94, 0.70)];
    let neck = add_chain(&neck_pts, (0..4).map(|i| format!("neck_{i}")).collect(), Some(spine[3]), 0.07, 0.07);
    let head = add_chain(&[v3(0.0, 0.94, 0.70), v3(0.0, 0.95, 0.92)], vec!["head".into()], Some(neck[3]), 0.09, 0.09)[0];
    let _jaw = add_chain(&[v3(0.0, 0.89, 0.74), v3(0.0, 0.86, 0.90)], vec!["jaw".into()], Some(head), 0.05, 0.03)[0];
    for (side, sx) in [("L", 1.0), ("R", -1.0)] {
        let _eye =
            add_chain(&[v3(0.05 * sx, 0.97, 0.82), v3(0.06 * sx, 0.97, 0.86)], vec![format!("eye.{side}")], Some(head), 0.02, 0.02)[0];
        let horn_pts = [v3(0.04 * sx, 1.00, 0.74), v3(0.08 * sx, 1.08, 0.70), v3(0.11 * sx, 1.15, 0.65), v3(0.13 * sx, 1.22, 0.60)];
        add_chain(&horn_pts, (0..3).map(|i| format!("horn_{i}.{side}")).collect(), Some(head), 0.02, 0.02);
    }
    let tail_pts: Vec<Vec3> = (0..=10).map(|i| v3(0.0, 0.58 - i as f64 * 0.02, -0.20 - i as f64 * 0.10)).collect();
    add_chain(&tail_pts, (0..10).map(|i| format!("tail_{i}")).collect(), Some(spine[0]), 0.04, 0.04);
    for (side, sx) in [("L", 1.0), ("R", -1.0)] {
        let leg_pts = [v3(0.12 * sx, 0.58, -0.10), v3(0.12 * sx, 0.30, -0.12), v3(0.12 * sx, 0.04, -0.10), v3(0.12 * sx, 0.02, 0.02)];
        add_chain(&leg_pts, (0..3).map(|i| format!("leg_{i}.{side}")).collect(), Some(spine[0]), 0.045, 0.045);
        let wing_pts = [
            v3(0.12 * sx, 0.66, 0.25),
            v3(0.32 * sx, 0.68, 0.23),
            v3(0.54 * sx, 0.70, 0.20),
            v3(0.76 * sx, 0.70, 0.17),
            v3(0.95 * sx, 0.69, 0.14),
        ];
        let wing = add_chain(&wing_pts, (0..4).map(|i| format!("wing_{i}.{side}")).collect(), Some(spine[2]), 0.04, 0.02);
        for f in 0..12 {
            let z_off = 0.04 * (f as f64 + 1.0);
            let bx = (0.30 + 0.055 * f as f64) * sx;
            add_chain(
                &[v3(bx, 0.68, 0.20 - z_off * 0.3), v3(bx + 0.04 * sx, 0.67, 0.12 - z_off)],
                vec![format!("feather_{f}.{side}")],
                Some(wing[(f / 3).min(wing.len() - 1)]),
                0.018,
                0.008,
            );
        }
    }
    assert!(bones.len() >= 55 && bones.len() <= 107, "bone count={}", bones.len());
    let path = temp_path("winged_creature_60bones.glb");
    std::fs::write(
        &path,
        to_glb(&Fixture {
            bones,
            body_pos: pos,
            body_tris: tris,
            body_w: w,
            cape_pos: Vec::new(),
            cape_tris: Vec::new(),
            cape_w: Vec::new(),
        })
        .to_bytes(),
    )
    .unwrap();
    let checked = check_full(&path, &CtxOpts::default(), &MotionSpec::default()).unwrap();
    assert_eq!(checked.model.skel.anatomy.body, Body::Legged);
    assert!(checked.model.skel.anatomy.roles.contains_key("wing*.L"));
    assert!(checked.model.skel.anatomy.roles.contains_key("jaw"));
    // The range of motion is a stress test: small bones (horns, eyes,
    // feathers) are posed like any other weighted bone, each ±45°, so a rig
    // that tears there shows it. Softening this hides tears (2026-11-24).
    let posed_45 = |token: &str| {
        checked.ctx.poses.iter().any(|p| {
            p.moves.iter().any(|&(j, _, rad)| checked.model.skel.names[j].contains(token) && (rad.to_degrees().abs() - 45.0).abs() < 1e-6)
        })
    };
    for token in ["horn", "eye", "feather", "tail", "neck"] {
        assert!(
            posed_45(token),
            "no ±45° pose on a {token} bone; poses: {:?}",
            checked.ctx.poses.iter().map(|p| &p.name).collect::<Vec<_>>()
        );
    }

    // Built-in creature gait synthesis produces walk, trot, lunge, attack clips on deep creatures cleanly.
    let gaits = check_full_with_gait(&path, &CtxOpts::default(), &MotionSpec::default(), true).unwrap();
    let clip_sec = gaits.rep.headline.iter().find(|s| s.name == "game clips").unwrap();
    assert_eq!(clip_sec.clips, vec!["walk", "trot", "lunge", "attack"]);
    assert_eq!(clip_sec.tears_pct, 0.0, "clean creature had gait clip tears: {:?}", clip_sec.findings);

    // Also test a deep 75-bone legless serpentine creature (`Body::Other`) -> zero 45-deg kinks & 0% tears.
    let mut s_bones = Vec::new();
    let mut s_pts = Vec::new();
    let mut s_chain = Vec::new();
    for i in 0..75 {
        s_pts.push(v3(0.0, 0.30, 1.5 - i as f64 * 0.05));
    }
    for i in 0..74 {
        s_bones.push(Bone {
            name: leak(format!("spine_seg_{i:02}")),
            parent: if i == 0 { None } else { Some(i - 1) },
            head: s_pts[i],
            tail: s_pts[i + 1],
        });
        s_chain.push((i as u32, i as f64 * 0.05));
    }
    let mut spos = Vec::new();
    let mut stris = Vec::new();
    let mut sw = Vec::new();
    add_tube(&mut spos, &mut stris, &mut sw, &s_pts, v3(1.0, 0.0, 0.0), 0.04, 10, 0.08, 0.08, &s_chain, 0.02);
    let spath = temp_path("serpent_75bones.glb");
    std::fs::write(
        &spath,
        to_glb(&Fixture {
            bones: s_bones,
            body_pos: spos,
            body_tris: stris,
            body_w: sw,
            cape_pos: Vec::new(),
            cape_tris: Vec::new(),
            cape_w: Vec::new(),
        })
        .to_bytes(),
    )
    .unwrap();
    let scheck = check_full(&spath, &CtxOpts::default(), &MotionSpec::default()).unwrap();
    assert_eq!(scheck.model.skel.anatomy.body, Body::Other);
    // Every weighted, non-root segment bends and twists ±45° on its own.
    let mut per_bone_45 = std::collections::BTreeSet::new();
    for p in &scheck.ctx.poses {
        for &(j, _, rad) in &p.moves {
            if p.moves.len() == 1 && (rad.to_degrees().abs() - 45.0).abs() < 1e-6 {
                per_bone_45.insert(j);
            }
        }
    }
    assert!(per_bone_45.len() >= 60, "only {} of 74 serpent bones get ±45° poses", per_bone_45.len());
}
