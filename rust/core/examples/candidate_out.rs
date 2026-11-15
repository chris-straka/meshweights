//! Writes one fix candidate's weights as a GLB, to check it alone:
//!     candidate_out in.glb out.glb crotch|mirror-left|mirror-right
use std::path::Path;
use weightforge_core as wf;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let m = wf::scene::Model::load(Path::new(&a[0])).unwrap();
    let w = match a[2].as_str() {
        "crotch" => wf::fix::crotch_to_pelvis(&m),
        "own-bones" => {
            let ctx = wf::metrics::Ctx::new(&m, &wf::metrics::CtxOpts { clips: false, ..Default::default() });
            Some(wf::fix::own_bones(&m, &ctx))
        }
        "mirror-left" => wf::fix::mirror(&m, true),
        "mirror-right" => wf::fix::mirror(&m, false),
        _ => None,
    }
    .expect("no such candidate for this rig");
    std::fs::write(&a[1], m.write_weights(&w).unwrap().to_bytes()).unwrap();
}
