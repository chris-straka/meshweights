use std::path::{Path, PathBuf};
use std::process::ExitCode;
use weightforge_core as wf;

use crate::args::{Args, ctx_opts, load_poses, motion_spec};
use crate::common::{fail, fails, write_json_pretty, write_png};

/// The ML candidate: SkinTokens weights for the input's own skeleton
/// (`skintokens skin` via `SKINTOKENS_BIN` or `PATH`). Err = the tool is
/// missing (usage error); Ok(None) = it ran and failed, so the fix goes on
/// without it (a warning, never a worse result).
fn skintokens_candidate(input: &str) -> Result<Option<wf::scene::Model>, String> {
    let bin = std::env::var_os("SKINTOKENS_BIN")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("skintokens")).find(|f| f.is_file())))
        .ok_or("skintokens not found (set SKINTOKENS_BIN or install skintokens on PATH)")?;
    if !bin.is_file() {
        return Err(format!("{} not found", bin.display()));
    }
    let dir = std::env::temp_dir().join(format!("weights_skintokens_{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let (out, report) = (dir.join("skintokens.glb"), dir.join("result.json"));
    eprintln!("weights: running SkinTokens on {input} (ML candidate)...");
    let status = std::process::Command::new(&bin)
        .arg("skin")
        .arg(input)
        .arg(&out)
        .arg("--report")
        .arg(&report)
        .stdout(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("{}: {e}", bin.display()))?;
    let model = if status.success() { wf::scene::Model::load(&out).ok() } else { None };
    if model.is_none() {
        let why = std::fs::read_to_string(&report).unwrap_or_default();
        eprintln!("weights: skintokens candidate skipped ({status}) {}", why.trim());
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(model)
}

pub fn cmd_fix(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("fix needs an input .glb") };
    let Some(out) = a.get("--out") else { return fail("fix needs --out <fixed.glb>") };
    let methods = match wf::fix::Method::parse(a.get("--method").unwrap_or("auto")) {
        Some(m) => m,
        None => return fail("--method must be auto, smooth, geodesic, transfer, or optimize"),
    };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let mut model = match wf::scene::Model::load(Path::new(input)) {
        Ok(m) => m,
        Err(e) => return fail(e),
    };
    // A body whose fused legs were cut apart (an upstream unfuse pass
    // records the cut): the repair starts from each half following its own
    // leg. The writer keeps the file's own bytes for every vertex whose
    // weights equal the loaded ones, so the file is written from an untouched
    // copy of the model, not from the one the repair starts on.
    let mut as_read = None;
    if let Some(w) = wf::unfuse::cut_start(&model) {
        let moved = (0..model.nverts()).filter(|&v| w[v] != model.weights[v]).count();
        eprintln!("weights: legs cut apart: {moved} vertices given to their own leg before the fix");
        as_read = match wf::scene::Model::load(Path::new(input)) {
            Ok(m) => Some(m),
            Err(e) => return fail(e),
        };
        model.weights = w;
    }
    let source = match a.get("--source").map(|p| wf::scene::Model::load(Path::new(p))) {
        Some(Ok(m)) => Some(m),
        Some(Err(e)) => return fail(format!("--source: {e}")),
        None => None,
    };
    let mut externals = Vec::new();
    let cand_paths = a.many("--candidate");
    let file_name = |p: &str| Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    for &p in &cand_paths {
        // Named by file; by folder and file when two share a file name
        // (heat/x.glb and voxel/x.glb), so the report says which won.
        let mut name = file_name(p);
        if cand_paths.iter().filter(|&&q| file_name(q) == name).count() > 1
            && let Some(dir) = Path::new(p).parent().and_then(|d| d.file_name())
        {
            name = format!("{}/{name}", dir.to_string_lossy());
        }
        match wf::scene::Model::load(Path::new(p)) {
            Ok(m) => externals.push((name, m)),
            Err(e) => return fail(format!("--candidate {p}: {e}")),
        }
    }
    if a.has("--skintokens") {
        match skintokens_candidate(input) {
            Ok(Some(m)) => externals.push(("skintokens".to_string(), m)),
            Ok(None) => {}
            Err(e) => return fail(format!("--skintokens: {e}")),
        }
    }
    let ctx = wf::metrics::Ctx::new(&model, &opts);
    // Clip evaluation for the headline guard.
    let spec = motion_spec(a);
    let motion_ctx = match wf::motion_for(&model, false, &spec) {
        Ok((Some(m), _)) => Some(ctx.with_poses(&model, m.poses)),
        Ok((None, notes)) => {
            for n in notes {
                eprintln!("weights: {n}");
            }
            if a.has("--gait") && spec.source.is_none() && model.skel.anatomy.body != wf::anatomy::Body::Humanoid {
                let gm = wf::motion::creature_gait_clips(&model, wf::motion::FPS);
                (!gm.poses.is_empty()).then(|| ctx.with_poses(&model, gm.poses))
            } else {
                None
            }
        }
        Err(e) => return fail(format!("--motion: {e}")),
    };
    let name = Path::new(input).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let fo = wf::fix::FixOpts {
        methods,
        source: source.as_ref(),
        external: externals.iter().map(|(n, m)| (n.clone(), m)).collect(),
        all_regions: a.has("--all-regions"),
        headline_guard: !a.has("--no-headline-guard"),
        motion: motion_ctx.as_ref(),
    };
    let res = wf::fix::fix(&name, &model, &ctx, &fo);
    let glb = match as_read.as_ref().unwrap_or(&model).write_weights(&res.weights) {
        Ok(g) => g,
        Err(e) => return fail(e),
    };
    let glb_bytes = match glb.try_to_bytes() {
        Ok(b) => b,
        Err(e) => return fail(format!("{out}: {e}")),
    };
    if let Err(e) = std::fs::write(out, glb_bytes) {
        return fail(format!("{out}: {e}"));
    }
    let report_path = a.get("--report").map(str::to_string).unwrap_or_else(|| {
        let p = Path::new(out);
        p.with_file_name(format!("{}.report.json", p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()))
            .to_string_lossy()
            .into_owned()
    });
    if let Err(e) = write_json_pretty(Path::new(&report_path), serde_json::to_string_pretty(&res.fix)) {
        return fail(e);
    }
    if let Some(sheet_path) = a.get("--sheet") {
        let (rb, ra) = (&res.report_before, &res.report_after);
        let ev0 = wf::metrics::evaluate(&model, &ctx, &model.weights);
        let (ma, mb) = (wf::report::finding_mask(&ctx, &ev0, rb), wf::report::finding_mask(&ctx, &res.eval, ra));
        let ta = format!("{name} (input)");
        let tb = Path::new(out).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let sa = wf::render::Side {
            model: &model,
            ctx: &ctx,
            ev: &ev0,
            w: &model.weights,
            info: wf::render::SheetInfo { title: &ta, mask: &ma, score: rb.score, tears: 0.0, pass: rb.pass, fails: fails(rb) },
        };
        let sb = wf::render::Side {
            model: &model,
            ctx: &ctx,
            ev: &res.eval,
            w: &res.weights,
            info: wf::render::SheetInfo { title: &tb, mask: &mb, score: ra.score, tears: 0.0, pass: ra.pass, fails: fails(ra) },
        };
        if let Err(e) = write_png(Path::new(sheet_path), &wf::render::compare(&sa, &sb, &wf::render::SheetOpts::default())) {
            return fail(e);
        }
    }
    let f = &res.fix;
    println!(
        "{} {name}: score {:.1} -> {:.1}, failing findings {} -> {}, {} verts changed",
        if f.after.pass {
            "FIXED"
        } else if f.improved {
            "IMPROVED"
        } else {
            "UNCHANGED"
        },
        f.before.score,
        f.after.score,
        f.before.fails,
        f.after.fails,
        f.verts_changed
    );
    if !f.tears_before.is_empty() {
        let line: Vec<String> =
            f.tears_before.iter().map(|(k, b)| format!("{k} {b:.2}% -> {:.2}%", f.tears_after.get(k).copied().unwrap_or(*b))).collect();
        println!("  torn: {}", line.join(", "));
    }
    for r in f.regions.iter().filter(|r| r.chosen != "original") {
        println!("  {:<24} {:>5.1} -> {:>5.1}  via {}", r.label, r.before, r.after, r.chosen);
    }
    println!("{out}\n{report_path}");
    if f.after.pass { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
