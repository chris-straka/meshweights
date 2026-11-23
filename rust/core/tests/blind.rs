//! The blind benchmark's planted bad case and game-camera pair: a
//! swapped mirrored limb must tear, and a pair sheet must show both
//! rigs whole at the same frame with only LEFT and RIGHT on it.

use weightforge_core::blind::{
    BlindSide, GameCamera, cell_image, claim_view, frame_share, game_camera_from_path, game_fit, game_frame_share_from_path, pair_sheet,
    planted_bad,
};
use weightforge_core::fixtures;
use weightforge_core::glb::Glb;
use weightforge_core::metrics::{Ctx, CtxOpts, evaluate};
use weightforge_core::owner;
use weightforge_core::paint;
use weightforge_core::report::build;
use weightforge_core::scene::Model;
use weightforge_core::skin::lbs;

fn load(fault: &str) -> Model {
    let bytes = fixtures::to_glb(&fixtures::build(fault)).to_bytes();
    Model::from_glb(Glb::parse(&bytes).unwrap()).unwrap()
}

#[test]
fn the_planted_swap_tears_and_the_pair_is_whole_body() {
    let m = load("clean");
    let ctx = Ctx::new(&m, &CtxOpts::default());
    let (w, pose, verts, what) = planted_bad(&m, &ctx, false).expect("the mannequin has mirrored arms");
    assert!(what.contains("swapped"), "{what}");
    assert!(!verts.is_empty());
    // Tears flag the planted rig; the clean rig tears nothing.
    let clean_torn = owner::torn(&m, &ctx, &m.weights, false);
    let planted_torn = owner::torn(&m, &ctx, &w, false);
    assert_eq!(clean_torn.iter().filter(|&&t| t).count(), 0);
    assert!(planted_torn.iter().filter(|&&t| t).count() >= 25, "{}", planted_torn.iter().filter(|&&t| t).count());
    // Its old score collapses too (the benchmark keeps both numbers;
    // the score is only retired where it disagrees, not everywhere).
    let ev = evaluate(&m, &ctx, &m.weights);
    let planted_ev = weightforge_core::metrics::evaluate_with(&m, &ctx, &w, false);
    assert!(build("planted", &m, &ctx, &planted_ev).score < build("clean", &m, &ctx, &ev).score);

    // The pair at the planted frame: same camera and crop, whole body
    // in frame, the planted side visibly unlike the clean side.
    let p = paint::read(&m);
    let good = BlindSide { model: &m, ctx: &ctx, weights: &m.weights, paint: &p };
    let bad = BlindSide { model: &m, ctx: &ctx, weights: &w, paint: &p };
    let mut marked = vec![false; m.nverts()];
    for &v in &verts {
        marked[v as usize] = true;
    }
    let view = claim_view(&bad, pose, &marked);
    let (pg, pb) = (lbs(&m, &ctx.mats[pose], &m.weights), lbs(&m, &ctx.mats[pose], &w));
    let share = frame_share(GameCamera { distance_m: 3.0, fov_deg: 75.0 });
    let fit = game_fit(&view, &[&pg, &pb], owner::height(&m), 520, 640, share);
    let sheet =
        pair_sheet(&cell_image(&good, pose, &view, fit, 520, 640, "LEFT"), &cell_image(&bad, pose, &view, fit, 520, 640, "RIGHT"), 1);
    assert_eq!((sheet.w, sheet.h), (1040, 640 + 52));
    assert_ne!(sheet.rgb, vec![0u8; sheet.rgb.len()]);
    // The two sides differ in a real share of pixels (the swap bends
    // an arm across), not in a label.
    let half = sheet.w / 2;
    let mut diff = 0usize;
    for y in 60..sheet.h - 40 {
        for x in 20..half - 20 {
            let l = (y * sheet.w + x) * 3;
            let r = (y * sheet.w + x + half) * 3;
            if sheet.rgb[l..l + 3] != sheet.rgb[r..r + 3] {
                diff += 1;
            }
        }
    }
    // 2000 pixels at the 45 degree camera assumed until 2026-11-19 (a body
    // 70% of the frame); the game's 75 degrees draws it at 38%, so a share
    // of the body's pixels is (0.38 / 0.70)^2 of what it was.
    let k = (share / 0.704).powi(2);
    assert!(diff as f64 > 2000.0 * k, "{diff} <= {}", 2000.0 * k);
}

#[test]
fn explicit_metrics_path_reads_camera_without_game_checkout() {
    let dir = std::env::temp_dir().join(format!("wf_blind_cam_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("metrics.ron");
    std::fs::write(&path, "// camera test\n{\"camera\": {\"distance\": 3.0, \"fov\": 75.0}}\n").unwrap();
    let cam = game_camera_from_path(&path).unwrap();
    assert_eq!(cam, GameCamera { distance_m: 3.0, fov_deg: 75.0 });
    assert!((game_frame_share_from_path(&path).unwrap() - 0.380).abs() < 0.005);
    assert!(game_camera_from_path(&dir.join("missing.ron")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn planted_candidates_swap_twist_helpers_and_dark_paint_clears_follow_rmse_gate() {
    use weightforge_core::blind::{is_upright_pose, planted_candidates};
    use weightforge_core::math::{Mat4, Quat, v3};

    let mut f = fixtures::build("clean");
    let ua_l = f.bones.iter().position(|b| b.name == "DEF-upper_arm.L").unwrap();
    let ua_r = f.bones.iter().position(|b| b.name == "DEF-upper_arm.R").unwrap();
    let sh_l = f.bones[ua_l].parent;
    let sh_r = f.bones[ua_r].parent;
    let tw_l = f.bones.len();
    f.bones.push(fixtures::Bone { name: "DEF-upper_arm_twist.L", parent: sh_l, head: f.bones[ua_l].head, tail: f.bones[ua_l].tail });
    let tw_r = f.bones.len();
    f.bones.push(fixtures::Bone { name: "DEF-upper_arm_twist.R", parent: sh_r, head: f.bones[ua_r].head, tail: f.bones[ua_r].tail });
    // Move all upper-arm weight onto the twist helper bones to verify helper mirroring.
    for vw in &mut f.body_w {
        for (j, _) in vw.iter_mut() {
            if *j as usize == ua_l {
                *j = tw_l as u32;
            } else if *j as usize == ua_r {
                *j = tw_r as u32;
            }
        }
    }

    let mut glb = fixtures::to_glb(&f);
    glb.json["materials"] = serde_json::json!([
        {"name": "dark_cloth", "pbrMetallicRoughness": {"baseColorFactor": [0.18, 0.20, 0.24, 1.0]}}
    ]);
    if let Some(meshes) = glb.json["meshes"].as_array_mut() {
        for m in meshes {
            if let Some(prims) = m["primitives"].as_array_mut() {
                for p in prims {
                    p["material"] = serde_json::json!(0);
                }
            }
        }
    }
    let bytes = glb.to_bytes();
    let m = Model::from_glb(Glb::parse(&bytes).unwrap()).unwrap();
    assert_eq!(m.skel.helpers.len(), 2);

    // Build a custom ctx with one supine collapse pose and one upright asymmetric pose.
    let mut ctx = Ctx::new(&m, &CtxOpts { clips: false, ..Default::default() });
    ctx.poses.truncate(2);
    ctx.mats.truncate(2);
    // Pose 0: supine 85-deg forward pitch at spine.
    let mut mats_supine = vec![Mat4::IDENTITY; m.njoints()];
    let rot_supine = Mat4::from_trs(v3(0.0, 0.0, 0.0), Quat::from_axis_angle(v3(1.0, 0.0, 0.0), 85f64.to_radians()), v3(1.0, 1.0, 1.0));
    for (j, mat) in mats_supine.iter_mut().enumerate() {
        let pivot = m.skel.head[0];
        *mat = Mat4::from_trs(pivot, Quat::IDENTITY, v3(1.0, 1.0, 1.0))
            * rot_supine
            * Mat4::from_trs(-pivot, Quat::IDENTITY, v3(1.0, 1.0, 1.0));
        let _ = j;
    }
    ctx.mats[0] = mats_supine;
    assert!(!is_upright_pose(&m, &ctx.mats[0]));
    assert!(is_upright_pose(&m, &ctx.mats[1]));

    let cands = planted_candidates(&m, &ctx, false);
    assert!(!cands.is_empty(), "expected at least one planted candidate");
    let c = &cands[0];
    assert!(c.torn >= 25, "twist-helper arm swap must tear >=25 verts, got {}", c.torn);
    assert_eq!(c.pose, 1, "planted candidate should prefer upright pose over supine pose 0");

    let p = paint::read(&m);
    let good = BlindSide { model: &m, ctx: &ctx, weights: &m.weights, paint: &p };
    let bad = BlindSide { model: &m, ctx: &ctx, weights: &c.weights, paint: &p };
    let mut marked = vec![false; m.nverts()];
    for &v in &c.vertices {
        marked[v as usize] = true;
    }
    let view = claim_view(&bad, c.pose, &marked);
    let (pg, pb) = (lbs(&m, &ctx.mats[c.pose], &m.weights), lbs(&m, &ctx.mats[c.pose], &c.weights));
    let share = frame_share(GameCamera { distance_m: 3.0, fov_deg: 75.0 });
    let fit = game_fit(&view, &[&pg, &pb], owner::height(&m), 520, 640, share);
    let ia = cell_image(&good, c.pose, &view, fit, 520, 640, "");
    let ib = cell_image(&bad, c.pose, &view, fit, 520, 640, "");
    let sq: f64 = ia.rgb.iter().zip(&ib.rgb).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum();
    let rmse = (sq / ia.rgb.len() as f64).sqrt();
    assert!(rmse > 5.0, "dark-painted follow camera planted RMSE must exceed 5.0, got {rmse:.2}");
}
