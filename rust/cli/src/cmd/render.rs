use std::path::Path;
use std::process::ExitCode;
use weightforge_core as wf;

use crate::args::{Args, ctx_opts, load_poses, load_poses_from, motion_spec, parse_kinds, parse_pos_usize, parse_range_usize, parse_usize};
use crate::common::{encode_frames, fail, fails, ffmpeg, write_json_pretty, write_png};

pub fn cmd_sheet(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("sheet needs an input .glb") };
    let Some(out) = a.get("--out") else { return fail("sheet needs --out <file.png>") };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let cols = match parse_range_usize(a, "--cols", 4, 1, 8) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let (model, ctx, ev, rep) = match wf::check(Path::new(input), &opts) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let so = wf::render::SheetOpts { cols, ..Default::default() };
    let mask = wf::report::finding_mask(&ctx, &ev, &rep);
    let info = wf::render::SheetInfo { title: &rep.file, mask: &mask, score: rep.score, tears: 0.0, pass: rep.pass, fails: fails(&rep) };
    let img = wf::render::sheet(&model, &ctx, &ev, &model.weights, &info, &so);
    if let Err(e) = write_png(Path::new(out), &img) {
        return fail(e);
    }
    println!("{out}");
    if rep.pass { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

/// A close-up sheet per headline finding (see SHOW in the usage).
pub fn cmd_show(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("show needs an input .glb") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("show needs --out <dir>") };
    let top = match parse_pos_usize(a, "--top", 6) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let kinds = match parse_kinds(a) {
        Ok(k) => k,
        Err(e) => return fail(e),
    };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let spec = motion_spec(a);
    let c = match wf::check_full(Path::new(input), &opts, &spec) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(format!("{}: {e}", dir.display()));
    }
    let paint = wf::paint::read(&c.model);
    let mut picked: Vec<(&wf::metrics::Ctx, &wf::owner::Finding, String)> = Vec::new();
    for sec in &c.rep.headline {
        let ctx = if sec.name == "game clips" { c.motion.as_ref().map(|m| &m.0).unwrap_or(&c.ctx) } else { &c.ctx };
        for f in &sec.findings {
            if kinds.as_ref().is_none_or(|k| k.contains(&f.kind)) {
                picked.push((ctx, f, sec.name.clone()));
            }
        }
    }
    picked.truncate(top);
    let mut index = Vec::new();
    for (i, (ctx, f, sec)) in picked.iter().enumerate() {
        let title = format!("{} - {sec} - weights show", c.rep.file);
        let img = wf::render::finding_sheet(&c.model, ctx, &c.model.weights, f, &paint, &title);
        let path = dir.join(format!("{:02}_{}.png", i + 1, f.kind));
        if let Err(e) = write_png(&path, &img) {
            return fail(e);
        }
        println!("{}", path.display());
        index.push(
            serde_json::json!({"file": path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), "section": sec, "finding": f}),
        );
    }
    let findings_path = dir.join("findings.json");
    if let Err(e) = write_json_pretty(&findings_path, serde_json::to_string_pretty(&index)) {
        return fail(e);
    }
    ExitCode::SUCCESS
}

/// The before rig's worst failing areas on the game's clips, each beside
/// the same place on the after rig at the same frame (BEFORE-AFTER).
pub fn cmd_before_after(a: &Args) -> ExitCode {
    let (Some(bp), Some(ap)) = (a.pos.get(1), a.pos.get(2)) else { return fail("before-after needs <before.glb> <after.glb>") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("before-after needs --out <dir>") };
    let top = match parse_pos_usize(a, "--top", 3) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let kinds = match parse_kinds(a) {
        Ok(k) => k,
        Err(e) => return fail(e),
    };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let spec = motion_spec(a);
    let (b, af) = match (wf::check_full(Path::new(bp), &opts, &spec), wf::check_full(Path::new(ap), &opts, &spec)) {
        (Ok(b), Ok(a)) => (b, a),
        (Err(e), _) | (_, Err(e)) => return fail(e),
    };
    let (Some(bm), Some(am)) = (b.motion.as_ref(), af.motion.as_ref()) else {
        return fail("before-after judges the game's clips: give --motion character.glb (or rigs with their own clips)");
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(format!("{}: {e}", dir.display()));
    }
    // The after rig's vertex at each before vertex's place (the same index
    // when the fix only changed weights).
    let same = b.model.nverts() == af.model.nverts() && b.model.rest.iter().zip(&af.model.rest).all(|(x, y)| (*x - *y).len() < 1e-6);
    let map = |v: u32| -> u32 {
        if same {
            return v;
        }
        let Some(&p) = b.model.rest.get(v as usize) else { return 0 };
        (0..af.model.nverts()).min_by(|&i, &j| (af.model.rest[i] - p).len().total_cmp(&(af.model.rest[j] - p).len())).unwrap_or(0) as u32
    };
    let torn_after = wf::owner::torn(&af.model, &am.0, &af.model.weights, true);
    let area_a = wf::owner::vertex_areas(&af.model);
    let k = wf::owner::BODY_M / wf::owner::height(&af.model);
    let (pb, pa) = (wf::paint::read(&b.model), wf::paint::read(&af.model));
    let mut picked: Vec<&wf::owner::Finding> = b
        .rep
        .headline
        .iter()
        .filter(|s| s.name == "game clips")
        .flat_map(|s| s.findings.iter())
        .filter(|f| f.severity == "fail" && kinds.as_ref().map_or(f.kind != "limb_through", |k| k.contains(&f.kind)))
        .collect();
    picked.sort_by(|x, y| y.area_cm2.total_cmp(&x.area_cm2));
    picked.truncate(top);
    if picked.is_empty() {
        println!("{bp}: nothing failing on the game's clips");
    }
    let mut index = Vec::new();
    for (i, f) in picked.iter().enumerate() {
        let mut after_vs: Vec<u32> = f.vertices.iter().map(|&v| map(v)).collect();
        after_vs.sort_unstable();
        after_vs.dedup();
        let left: f64 = after_vs
            .iter()
            .filter(|&&v| torn_after.get(v as usize).copied().unwrap_or(false))
            .map(|&v| area_a.get(v as usize).copied().unwrap_or(0.0) * k * k * 1e4)
            .sum::<f64>()
            + 0.0;
        let mut marked_b = vec![false; b.model.nverts()];
        for &v in &f.vertices {
            if let Some(slot) = marked_b.get_mut(v as usize) {
                *slot = true;
            }
        }
        let bs =
            wf::render::BaSide { model: &b.model, ctx: &bm.0, marked: &marked_b, paint: &pb, label: format!("before, in {}", f.worst) };
        let as_ = wf::render::BaSide { model: &af.model, ctx: &am.0, marked: &torn_after, paint: &pa, label: "after, same frame".into() };
        let head0 = format!("{}: {}", f.kind.replace('_', " "), f.part);
        let head1 = format!("before: {} - {:.0} cm2", f.detail.replace('\u{b2}', "2"), f.area_cm2);
        let head2 = format!("after: {left:.0} cm2 of that skin still tears (red: all skin the after rig tears)");
        let title = format!("{} -> {} - game clips - weights before-after", b.rep.file, af.rep.file);
        let img = wf::render::before_after_sheet(&bs, &as_, &f.vertices, &after_vs, f.worst_pose, [&head0, &head1, &head2], &title);
        let path = dir.join(format!("{:02}_{}.png", i + 1, f.kind));
        if let Err(e) = write_png(&path, &img) {
            return fail(e);
        }
        println!("{}  before {:.0} cm2, after {left:.0} cm2", path.display(), f.area_cm2);
        index.push(
            serde_json::json!({"file": path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), "finding": f, "after_cm2": left}),
        );
    }
    let ba_path = dir.join("before_after.json");
    if let Err(e) = write_json_pretty(&ba_path, serde_json::to_string_pretty(&index)) {
        return fail(e);
    }
    ExitCode::SUCCESS
}

/// Loops of one rig for a person to judge (see PROBE in the usage).
pub fn cmd_probe(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("probe needs an input .glb") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("probe needs --out <dir>") };
    let first = match parse_usize(a, "--number-from", 1) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let (model, ctx, ev, rep) = match wf::check(Path::new(input), &opts) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let mask = wf::report::finding_mask(&ctx, &ev, &rep);
    let side = wf::render::Side {
        model: &model,
        ctx: &ctx,
        ev: &ev,
        w: &model.weights,
        info: wf::render::SheetInfo { title: &rep.file, mask: &mask, score: rep.score, tears: 0.0, pass: rep.pass, fails: fails(&rep) },
    };
    let o = wf::render::ZoomOpts { cell_w: 720, cell_h: 560, ..Default::default() };
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(format!("{}: {e}", dir.display()));
    }
    let cases = wf::render::probe_cases(&side, &o, &[1.8, 2.2, 2.6, 3.2, 4.5]);
    for (k, c) in cases.iter().enumerate() {
        let n = first + k;
        let frames = dir.join(format!("case_{n:02}"));
        let mut err = None;
        wf::render::probe_clip(&side, c, n, &o, |f, img| {
            let res = std::fs::create_dir_all(&frames)
                .map_err(|e| format!("{}: {e}", frames.display()))
                .and_then(|_| write_png(&frames.join(format!("{f:03}.png")), img));
            if let Err(e) = res {
                err.get_or_insert(e);
            }
        });
        if let Some(e) = err {
            return fail(e);
        }
        let mp4 = dir.join(format!("case_{n:02}.mp4"));
        let file = if encode_frames(&frames, &mp4) { mp4 } else { frames };
        let stretch = (c.stretch * 100.0).round() / 100.0;
        println!(
            "{}",
            serde_json::json!({"case": n, "file": file.to_string_lossy(), "rig": rep.file, "pose": c.name, "stretch": stretch, "bad": c.bad, "score": rep.score})
        );
    }
    ExitCode::SUCCESS
}

/// `compare --zoom --clips`: each row's loop as PNG frames, then as an
/// mp4 when ffmpeg is there (the frames are removed once it is written),
/// and a reel of every loop played twice.
fn write_clips(dir: &Path, sa: &wf::render::Side, sb: &wf::render::Side, o: &wf::render::ZoomOpts) -> Result<(), String> {
    let io = |e: std::io::Error| format!("{}: {e}", dir.display());
    std::fs::create_dir_all(dir).map_err(io)?;
    let mut rows: Vec<String> = Vec::new();
    let mut err = None;
    wf::render::compare_zoom_clips(sa, sb, o, |r, name, f, img| {
        if r == rows.len() {
            let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect();
            rows.push(format!("{:02}_{safe}", r + 1));
        }
        let d = dir.join(&rows[r]);
        let res = std::fs::create_dir_all(&d).map_err(io).and_then(|_| write_png(&d.join(format!("{f:03}.png")), img));
        if let Err(e) = res {
            err.get_or_insert(e);
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    let mut made = Vec::new();
    for row in &rows {
        let (frames, mp4) = (dir.join(row), dir.join(format!("{row}.mp4")));
        if encode_frames(&frames, &mp4) {
            println!("{}", mp4.display());
            made.push(format!("{row}.mp4"));
        } else {
            eprintln!("weights: no mp4 for {row} (is ffmpeg on PATH?); its frames are in {}", frames.display());
        }
    }
    if !made.is_empty() {
        let list = dir.join("reel.txt");
        let text: String = made.iter().flat_map(|m| [m, m]).map(|m| format!("file '{m}'\n")).collect();
        std::fs::write(&list, text).map_err(io)?;
        let reel = dir.join("reel.mp4");
        if ffmpeg(&[
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            &list.to_string_lossy(),
            "-c",
            "copy",
            "-movflags",
            "+faststart",
            &reel.to_string_lossy(),
        ]) {
            println!("{}", reel.display());
        }
        let _ = std::fs::remove_file(&list);
    }
    Ok(())
}

pub fn cmd_compare(a: &Args) -> ExitCode {
    let (Some(pa), Some(pb)) = (a.pos.get(1), a.pos.get(2)) else { return fail("compare needs two .glb files") };
    let Some(out) = a.get("--out") else { return fail("compare needs --out <file.png>") };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let set_b = match load_poses_from(a, "--poses-b") {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts_b = match ctx_opts(a, set_b.as_ref().or(set.as_ref())) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let rows = match parse_range_usize(a, "--rows", 5, 1, 12) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let ra = wf::check(Path::new(pa), &opts);
    let rb = wf::check(Path::new(pb), &opts_b);
    let ((ma, ca, ea, rpa), (mb, cb, eb, rpb)) = match (ra, rb) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => return fail(e),
    };
    let mask_a = wf::report::finding_mask(&ca, &ea, &rpa);
    let mask_b = wf::report::finding_mask(&cb, &eb, &rpb);
    // The zoomed sheet's verdict is by tears (what a person notices),
    // on the same poses the sheet shows; the score only backs it up.
    let tears_a = wf::owner::assess(&ma, &ca, &ea, &rpa, &ma.weights, false, "poses", Vec::new(), String::new()).tears_pct;
    let tears_b = wf::owner::assess(&mb, &cb, &eb, &rpb, &mb.weights, false, "poses", Vec::new(), String::new()).tears_pct;
    let sa = wf::render::Side {
        model: &ma,
        ctx: &ca,
        ev: &ea,
        w: &ma.weights,
        info: wf::render::SheetInfo {
            title: &rpa.file,
            mask: &mask_a,
            score: rpa.score,
            tears: tears_a,
            pass: rpa.pass,
            fails: fails(&rpa),
        },
    };
    let sb = wf::render::Side {
        model: &mb,
        ctx: &cb,
        ev: &eb,
        w: &mb.weights,
        info: wf::render::SheetInfo {
            title: &rpb.file,
            mask: &mask_b,
            score: rpb.score,
            tears: tears_b,
            pass: rpb.pass,
            fails: fails(&rpb),
        },
    };
    let worst = |r: &wf::report::Report| {
        r.regions
            .iter()
            .max_by_key(|x| x.bad)
            .filter(|x| x.bad > 0)
            .map(|x| format!("WORST PART: {} ({} BAD VERTS)", x.label, x.bad))
            .unwrap_or_default()
    };
    let (note_a, note_b) = (worst(&rpa), worst(&rpb));
    let zoom = wf::render::ZoomOpts {
        rows,
        label_a: a.get("--label-a").unwrap_or("A"),
        label_b: a.get("--label-b").unwrap_or("B"),
        note_a: &note_a,
        note_b: &note_b,
        ..Default::default()
    };
    let img = if a.has("--zoom") {
        wf::render::compare_zoom(&sa, &sb, &zoom)
    } else {
        wf::render::compare(&sa, &sb, &wf::render::SheetOpts::default())
    };
    if let Some(dir) = a.get("--clips") {
        if !a.has("--zoom") {
            return fail("--clips needs --zoom");
        }
        if let Err(e) = write_clips(Path::new(dir), &sa, &sb, &zoom) {
            return fail(e);
        }
    }
    if let Err(e) = write_png(Path::new(out), &img) {
        return fail(e);
    }
    println!("{out}");
    ExitCode::SUCCESS
}
