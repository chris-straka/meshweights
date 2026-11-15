//! For a cut body: the first wrong-bone finding's vertices, their cut side,
//! lateral position and height against the hip joints.
use std::path::Path;
use weightforge_core as wf;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let m = wf::scene::Model::load(Path::new(&a[0])).unwrap();
    let ctx = wf::metrics::Ctx::new(&m, &wf::metrics::CtxOpts { clips: false, ..Default::default() });
    let ev = wf::metrics::evaluate(&m, &ctx, &m.weights);
    let rep = wf::report::build("x", &m, &ctx, &ev);
    let sec = wf::owner::assess(&m, &ctx, &ev, &rep, &m.weights, false, "poses", vec![], String::new());
    let f = sec.findings.iter().find(|f| f.kind == "wrong_bone").unwrap();
    let legs = wf::unfuse::Legs::of(&m).unwrap();
    let side = wf::unfuse::outline_side(&m, &legs);
    let sk = &m.skel;
    let top = sk.head[sk.anatomy.roles["thigh.L"][0]].y.max(sk.head[sk.anatomy.roles["thigh.R"][0]].y);
    let left = sk.anatomy.up.cross(sk.anatomy.forward).normalized();
    let mid = sk.head[sk.anatomy.roles["hips"][0]].dot(left);
    let mut s = [0usize; 3];
    let (mut above, mut lpos) = (0, 0);
    for &v in &f.vertices {
        s[side[v as usize] as usize] += 1;
        if m.rest[v as usize].y >= top {
            above += 1;
        }
        if m.rest[v as usize].dot(left) - mid > 0.0 {
            lpos += 1;
        }
    }
    let wsum: std::collections::BTreeMap<String, f64> = f
        .vertices
        .iter()
        .flat_map(|&v| m.weights[v as usize].iter().map(|&(j, x)| (sk.names[j as usize].clone(), x)))
        .fold(Default::default(), |mut acc, (n, x)| {
            *acc.entry(n).or_default() += x;
            acc
        });
    let mut xs: Vec<(f64, f64)> =
        f.vertices.iter().map(|&v| ((m.rest[v as usize].dot(left) - mid) * 100.0, (m.rest[v as usize].y - top) * 100.0)).collect();
    xs.sort_by(|a, b| a.0.total_cmp(&b.0));
    println!("lateral cm / height below hip joints cm: {:?}", xs.iter().map(|(a, b)| format!("{a:.1}/{b:.1}")).collect::<Vec<_>>());
    println!("crotch {:?} top {top:.3} slack {:.3}", wf::unfuse::cut_crotch(&m), wf::unfuse::SIDE_SLACK * 1.75);
    println!(
        "{} verts; side none/L/R {:?}; above hip joints {}; on the left of the midline {}; weights {:?}",
        f.vertices.len(),
        s,
        above,
        lpos,
        wsum
    );
}
