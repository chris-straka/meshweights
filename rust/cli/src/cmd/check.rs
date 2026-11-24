use std::path::Path;
use std::process::ExitCode;
use weightforge_core as wf;

use crate::args::{Args, ctx_opts, load_poses, motion_spec};
use crate::common::{fail, print_json_pretty, write_json};

pub fn cmd_check(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("check needs an input .glb") };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let spec = motion_spec(a);
    let checked = match wf::check_full_with_gait(Path::new(input), &opts, &spec, a.has("--gait")) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let wf::Checked { model, ctx, ev, rep, .. } = checked;
    if let Some(vj) = a.get("--vertex-json") {
        let mask = wf::report::finding_mask(&ctx, &ev, &rep);
        let meshes: Vec<serde_json::Value> = model
            .raw_meshes()
            .into_iter()
            .map(|(mesh, node, pos, map)| {
                serde_json::json!({
                    "mesh": mesh,
                    "node": node,
                    "positions": pos.iter().flatten().collect::<Vec<_>>(),
                    "flags": map.iter().map(|&w| mask[w as usize]).collect::<Vec<_>>(),
                    "energy": map.iter().map(|&w| (ev.energy[w as usize] * 1e4).round() / 1e4).collect::<Vec<_>>(),
                })
            })
            .collect();
        let flag_names: serde_json::Map<String, serde_json::Value> =
            wf::metrics::FLAG_NAMES.iter().map(|(b, n)| (n.to_string(), serde_json::json!(b))).collect();
        let doc = serde_json::json!({"flag_bits": flag_names, "meshes": meshes});
        if let Err(e) = write_json(Path::new(vj), serde_json::to_vec(&doc)) {
            return fail(e);
        }
    }
    if a.has("--json") {
        if let Err(e) = print_json_pretty(serde_json::to_string_pretty(&rep)) {
            return fail(e);
        }
    } else {
        print!("{}", wf::report::text(&rep));
    }
    if rep.pass { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

/// How close a rig's weights come to a pro rig's (see CLOSENESS).
pub fn cmd_closeness(a: &Args) -> ExitCode {
    let (Some(pp), Some(tp)) = (a.pos.get(1), a.pos.get(2)) else { return fail("closeness needs <pro.glb> <test.glb>") };
    let (pro, test) = match (wf::scene::Model::load(Path::new(pp)), wf::scene::Model::load(Path::new(tp))) {
        (Ok(p), Ok(t)) => (p, t),
        (Err(e), _) | (_, Err(e)) => return fail(e),
    };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => wf::metrics::CtxOpts { clips: false, ..o },
        Err(e) => return fail(e),
    };
    let ctx = wf::metrics::Ctx::new(&pro, &opts);
    let spec = motion_spec(a);
    let (mv, notes) = match wf::motion_for(&pro, true, &spec) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    for n in notes {
        eprintln!("weights: {n}");
    }
    let mctx = mv.map(|m| ctx.with_poses(&pro, m.poses));
    let mut groups: Vec<(&str, &wf::metrics::Ctx)> = Vec::new();
    if let Some(m) = &mctx {
        groups.push(("game clips", m));
    }
    groups.push(("poses", &ctx));
    let c = match wf::closeness::closeness(&pro, &test, &groups) {
        Ok(c) => c,
        Err(e) => return fail(e),
    };
    if a.has("--json") {
        if let Err(e) = print_json_pretty(serde_json::to_string_pretty(&c)) {
            return fail(e);
        }
    } else {
        println!("{tp} against {pp}: weights {:.3} off (0 same, 1 disjoint)", c.weight_err);
        for m in &c.motion {
            println!(
                "  {} ({} frames): {:.2} cm mean, {:.2} cm at the 95th percentile, {:.2}% of the skin ever over 2 cm off",
                m.group, m.frames, m.mean_cm, m.p95_cm, m.off_pct
            );
        }
    }
    ExitCode::SUCCESS
}
