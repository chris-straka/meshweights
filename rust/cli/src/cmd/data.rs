use std::path::{Path, PathBuf};
use std::process::ExitCode;
use weightforge_core as wf;

use crate::args::Args;
use crate::common::{fail, write_json};

pub fn cmd_dump(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("dump needs an input .glb") };
    let Some(out) = a.get("--out") else { return fail("dump needs --out <weights.json>") };
    let model = match wf::scene::Model::load(Path::new(input)) {
        Ok(m) => m,
        Err(e) => return fail(e),
    };
    let meshes: Vec<serde_json::Value> = model
        .raw_meshes()
        .into_iter()
        .map(|(mesh, node, pos, map)| {
            serde_json::json!({
                "mesh": mesh,
                "node": node,
                "positions": pos.iter().flatten().collect::<Vec<_>>(),
                "influences": map.iter().map(|&w| model.weights[w as usize].iter().map(|&(j, x)| serde_json::json!([j, (x * 1e6).round() / 1e6])).collect::<Vec<_>>()).collect::<Vec<_>>(),
            })
        })
        .collect();
    let doc = serde_json::json!({"joints": model.skel.names, "meshes": meshes});
    if let Err(e) = write_json(Path::new(out), serde_json::to_vec(&doc)) {
        return fail(e);
    }
    println!("{out}");
    ExitCode::SUCCESS
}

pub fn cmd_fixture(a: &Args) -> ExitCode {
    let Some(which) = a.pos.get(1) else { return fail("fixture needs a fault name or `all`") };
    let Some(out) = a.get("--out") else { return fail("fixture needs --out <dir>") };
    let names: Vec<&str> = if which == "all" {
        wf::fixtures::FAULTS.to_vec()
    } else if wf::fixtures::FAULTS.contains(&which.as_str()) {
        vec![which.as_str()]
    } else {
        return fail(format!("unknown fixture {which} (have: {})", wf::fixtures::FAULTS.join(", ")));
    };
    if let Err(e) = std::fs::create_dir_all(out) {
        return fail(format!("{out}: {e}"));
    }
    for n in names {
        let path = PathBuf::from(out).join(format!("mannequin_{n}.glb"));
        let glb = wf::fixtures::to_glb(&wf::fixtures::build(n));
        let bytes = match glb.try_to_bytes() {
            Ok(b) => b,
            Err(e) => return fail(format!("{}: {e}", path.display())),
        };
        if let Err(e) = std::fs::write(&path, bytes) {
            return fail(format!("{}: {e}", path.display()));
        }
        println!("{}", path.display());
    }
    ExitCode::SUCCESS
}
