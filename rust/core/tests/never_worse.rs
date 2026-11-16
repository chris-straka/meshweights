//! `fix` never returns a rig that tears more skin than it came in with, on
//! any pose group the check reports (character line, 2026-11-14: a fix
//! kept a worse rig), and never breaks `pass` or adds failing deformation
//! findings when choosing headline candidates (P4 cast blind bench,
//! 2026-11-16: uncapped tear overrides traded wrong-bone drag for stretched
//! flat bridges across uncut skirts and crotches).

use weightforge_core::fix::{FixOpts, fix};
use weightforge_core::fixtures;
use weightforge_core::metrics::{Ctx, CtxOpts};
use weightforge_core::scene::{Model, normalize_vw};

#[test]
fn fix_never_tears_more_or_breaks_pass() {
    for fault in fixtures::FAULTS {
        let m = Model::from_glb(fixtures::to_glb(&fixtures::build(fault))).unwrap();
        let ctx = Ctx::new(&m, &CtxOpts::default());
        let res = fix(fault, &m, &ctx, &FixOpts::default());
        let f = &res.fix;
        assert!(!f.tears_before.is_empty(), "{fault}: tears not reported");
        assert!(!(f.before.pass && !f.after.pass), "{fault}: passing input turned into failing output");
        for g in f.findings_after.iter().filter(|g| g.severity == "fail") {
            assert!(
                f.findings_before.iter().any(|b| b.severity == "fail" && b.region == g.region),
                "{fault}: region {} gained failing finding {}",
                g.region,
                g.code
            );
        }
        for (k, b) in &f.tears_before {
            let a = f.tears_after[k];
            assert!(!b.is_sign_negative() && !a.is_sign_negative(), "{fault}: signed -0.0 in {k}");
            assert!(a <= *b + 1e-9, "{fault}: {k} {b} -> {a}");
        }
    }
}

#[test]
fn headline_choice_never_trades_pass_for_stretched_bridge() {
    let mut base = fixtures::build("clean");
    let tl = base.bones.iter().position(|b| b.name == "DEF-thigh.L").unwrap() as u32;
    let tr = base.bones.iter().position(|b| b.name == "DEF-thigh.R").unwrap() as u32;
    // Give inner-thigh vertices a mild cross-leg share that still passes
    // the deformation gate while registering wrong-bone drag in `torn`.
    for (v, p) in base.body_pos.iter().enumerate() {
        if (0.55..0.88).contains(&p.y) && p.x.abs() < 0.14 {
            let mut vw = base.body_w[v].clone();
            let other = if p.x >= 0.0 { tr } else { tl };
            vw.push((other, 0.28));
            base.body_w[v] = normalize_vw(vw);
        }
    }
    let m = Model::from_glb(fixtures::to_glb(&base)).unwrap();
    let ctx = Ctx::new(&m, &CtxOpts::default());
    // Offer a candidate that tears less on wrong_bone by forcing a hard
    // step that fails D_STRETCH on the elbow.
    let bad_cand = Model::from_glb(fixtures::to_glb(&fixtures::build("no_elbow_falloff"))).unwrap();
    let res = fix("bridge", &m, &ctx, &FixOpts { external: vec![("hard".into(), &bad_cand)], ..FixOpts::default() });
    assert!(
        !(res.fix.before.pass && !res.fix.after.pass),
        "headline choice broke pass: {} -> {}",
        res.fix.before.score,
        res.fix.after.score
    );
    for g in res.fix.findings_after.iter().filter(|g| g.severity == "fail") {
        assert!(
            res.fix.findings_before.iter().any(|b| b.severity == "fail" && b.region == g.region),
            "region {} gained failing finding {}",
            g.region,
            g.code
        );
    }
}
