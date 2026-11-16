//! The blind benchmark's planted bad case and game-camera pair: a
//! swapped mirrored limb must tear, and a pair sheet must show both
//! rigs whole at the same frame with only LEFT and RIGHT on it.

use weightforge_core::blind::{BlindSide, cell_image, claim_view, game_fit, pair_sheet, planted_bad};
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
    let fit = game_fit(&view, &[&pg, &pb], owner::height(&m), 520, 640);
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
    assert!(diff > 2000, "{diff}");
}
