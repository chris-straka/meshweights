//! `fix` never returns a rig that tears more skin than it came in with, on
//! any pose group the check reports (character line, 2026-11-14: a fix
//! kept a worse rig).

use weightforge_core::fix::{FixOpts, fix};
use weightforge_core::fixtures;
use weightforge_core::metrics::{Ctx, CtxOpts};
use weightforge_core::scene::Model;

#[test]
fn fix_never_tears_more() {
    for fault in fixtures::FAULTS {
        let m = Model::from_glb(fixtures::to_glb(&fixtures::build(fault))).unwrap();
        let ctx = Ctx::new(&m, &CtxOpts::default());
        let res = fix(fault, &m, &ctx, &FixOpts::default());
        let f = &res.fix;
        assert!(!f.tears_before.is_empty(), "{fault}: tears not reported");
        for (k, b) in &f.tears_before {
            let a = f.tears_after[k];
            assert!(a <= *b + 1e-9, "{fault}: {k} {b} -> {a}");
        }
    }
}
