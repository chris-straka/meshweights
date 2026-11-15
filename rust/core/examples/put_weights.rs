//! Puts another rigged GLB's weights on this one, through the fix's own
//! candidate transfer (joints matched by name, role, then position; skin by
//! surface): the result keeps this file's mesh and skeleton, so a check of
//! it compares weights alone.
//!     put_weights in.glb other.glb out.glb
use std::path::Path;
use weightforge_core as wf;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let m = wf::scene::Model::load(Path::new(&a[0])).unwrap();
    let other = wf::scene::Model::load(Path::new(&a[1])).unwrap();
    let ctx = wf::metrics::Ctx::new(&m, &wf::metrics::CtxOpts { clips: false, ..Default::default() });
    let mut w = wf::fix::external(&m, &ctx, &other, 0.03);
    m.lock_rigid(&mut w);
    std::fs::write(&a[2], m.write_weights(&w).unwrap().to_bytes()).unwrap();
}
