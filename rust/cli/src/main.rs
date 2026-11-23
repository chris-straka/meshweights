//! `weights`: find and fix bad skin weights on rigged GLB characters.
//! Exit codes: 0 = clean / success, 1 = deformation faults found,
//! 2 = usage or IO error (same shape as rfcheck).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use weightforge_core as wf;
use wf::metrics::CtxOpts;

const HELP: &str = "\
weights — skin-weight QA and repair for rigged GLB characters

USAGE:
  weights check   <in.glb> [--motion hero.glb] [--motion-clips walk,run,...] [--poses rom.ron]
                  [--json] [--no-clips] [--voxels N] [--vertex-json flags.json]
  weights fix     <in.glb> --out <fixed.glb> [--method auto|smooth|geodesic|transfer|optimize]
                  [--source base.glb] [--candidate other.glb]... [--skintokens]
                  [--all-regions] [--sheet before_after.png] [--motion hero.glb]
                  [--motion-clips ...] [--no-headline-guard]
  weights show    <in.glb> --out <dir> [--motion hero.glb] [--motion-clips ...] [--kind K,...]
                  [--top N]   (a close-up sheet per headline finding)
  weights before-after <before.glb> <after.glb> --out <dir> --motion hero.glb [--top N]
                  [--kind K,...]   (the before rig's worst areas on the game's clips,
                  each beside the same place on the after rig at the same frame)
  weights sheet   <in.glb> --out sheet.png [--poses rom.ron] [--cols N]
  weights compare <a.glb> <b.glb> --out ab.png [--poses rom.ron]
                  [--zoom [--rows N] [--label-a NAME] [--label-b NAME] [--clips DIR]]
                  [--poses-b rom_b.ron]
  weights probe   <in.glb> --out <dir> [--poses rom.ron] [--number-from N]
  weights blind   <a.glb> <b.glb> --out <dir> [--motion hero.glb] [--motion-clips ...]
                  [--seed N] [--top N] [--key key.json] [--camera follow|talk]
                  [--metrics metrics.ron] [--through]
                  (same-frame blind pairs per claim, at the game camera: the
                  follow camera's lens and distance from --metrics, $METRICS_FILE,
                  or $HLH_METRICS; --through: limbs passing through each other are claims too)
  weights blind-score <dir> --answers answers.json [--key key.json]
                  (lay a judge's answers against tears and the old score)
  weights closeness <pro.glb> <test.glb> [--motion hero.glb] [--json]
                  (how close test's weights come to a pro rig's, same mesh and skeleton)
  weights dump    <in.glb> --out weights.json   (per-vertex joints/weights, for DCC import)
  weights fixture <fault|all> --out <dir>

CHECK:
  The headline is what a person watching the character notices, on the
  game's clips and on the range-of-motion poses apart: tears (the share of
  the surface inside failing findings) and each finding as an area, named
  by part and side: cloth moving with a bone it doesn't belong to
  (wrong_bone), a limb's surface not following the limb (not_following),
  a stiff part (belt, buckle, pouch, plate) bending (stiff_bends), a limb
  passing through another (limb_through; a warning in the poses, whose full
  bends poke), stretch past 3.5x, collapse, bleed, visible speckle. Amounts
  are for a body 1.75 m tall. The 0-100 score follows as the older measure.
  Bone roles come from the skeleton's structure, not bone names.
  --motion <glb>   play this rig's clips (the hero's file) on the checked rig,
                   put on by bone role (nothing is written); default: the
                   checked file's own clips, if any
  --motion-clips   which clips, comma-separated (default with --motion:
                   walk,run,attack_1,dodge,hit,death; without: all the
                   file's own)

CLOSENESS:
  The benchmark's main score: test.glb's weights against a professional
  rig's (pro.glb, the same mesh and skeleton; joints by name, vertices by
  index or nearest rest position). Motion: every vertex posed with both
  through the poses and --motion's clips (default: pro.glb's own clips if
  any), on a body 1.75 m tall: mean cm, 95th percentile cm of each
  vertex's worst frame, and % of the skin ever more than 2 cm off.
  Weights: mean half-L1 distance per vertex (0 same, 1 disjoint).

SHOW:
  One PNG per headline finding, the largest first (--top, default 6;
  --kind keeps those kinds, e.g. wrong_bone,stiff_bends): the whole body
  in the frame where it is worst, then a close-up of it there and at rest
  from the same camera, in the character's paint, the finding in red.
  Clip findings come first, then the poses'. Files: <dir>/NN_<kind>.png,
  and <dir>/findings.json with what each one shows.

COMPARE:
  --zoom        the sheet for judging two rigs at a glance: a headline
                saying which is better, then one pose a row, the pose with
                the most failing vertices on either rig first (--rows,
                default 5), both rigs from the same camera and cropped to
                where they fail, grey clay under a side light with the
                failing faces solid red; a last row shows where each rig
                loses its score
  --clips <dir> with --zoom: a two-second loop for each row (both rigs from
                rest into the pose and back, same camera and crop) as
                <dir>/<rank>_<pose>.mp4, and <dir>/reel.mp4 with every loop
                twice. Needs ffmpeg on PATH; without it the frames stay in
                <dir>/<rank>_<pose>/ as PNG
  --label-a/-b  what to call each rig on the sheet (default A and B)
  --poses-b     the second rig's own pose set, when its bones are named
                differently (poses are matched by name)

BLIND:
  One pair per claim: for every failing headline finding of either rig
  (a claim, named by kind and part), both rigs at the frame where the
  claim is worst, from the game camera (the game's follow camera, its
  distance and lens read from the game's metrics.ron: 3 m and 75 degrees,
  the whole body about 38% of the frame; never a close-up of the fault), in the character's paint,
  nothing marked and no rig named. Renders each side cleanly and delegates
  pair shuffling, LEFT/RIGHT coin-flip (`--seed`, default 20261115),
  sheet compositing (`pair-NN.png`, `pair-NN-left.png`, `pair-NN-right.png`,
  `judge.json`), can't-tell control, planted bad case, and external
  `<dir>.key.json` anti-leak to `gk blind make` (`GK_BIN`, `PATH`, or
  `_tools/gk`).
  --camera talk draws from the game's talk camera instead (the close
  shot: 1.3 m from the face, a 38-degree lens, 24 degrees off the front,
  head, shoulders and chest in frame): only claims whose skin is in that
  frame are kept (the key lists the others as out_of_frame), and one
  more pair shows both rigs' faces at the first frame.

BLIND-SCORE:
  Answers are JSON: one object mapping pair-01, pair-02 (or pair_01)...
  to left, right or cant_tell (an 'answers' object holding that map works
  too). Validates the run via `gk blind score`, then lays each claim
  against the tears measure (kept) and the old 0-100 score: where the
  judge and tears agree against the score, that claim's score verdict is
  retired. Writes <dir>/verdict.json and prints the same. Exit 0 only
  when the control was answered can't-tell, the planted case was picked
  out, and tears flag the planted rig.

PROBE:
  Loops for a person to judge, so the pass line can be set to their eye:
  up to five poses of the rig spread over a range of stretch, each a
  two-second close-up in clay with only a case number on it (nothing
  marked, no verdict), as <dir>/case_NN.mp4 (ffmpeg; else PNG frames).
  Prints one JSON line a case with the numbers the judgement is laid
  against. --number-from continues the numbering across rigs.

FIX:
  Builds candidates (the input is always candidate 0), measures each one
  on sets of failing regions (only their flagged area changes, blended
  into the input at seams; bands measured in limb radii, not edge rings),
  and picks the plan that passes the gate if any can, then scores best,
  by an exact search: a better candidate never picks a worse plan. Never
  writes anything worse than the input, and no region gains a failing
  finding. Writes <out>.report.json (or --report path) with before/after
  numbers (`pick_exact`: false if the search hit its node budget).
  --method      auto (every method, default), or one or more of
                smooth,geodesic,transfer,optimize (comma-separated)
  --source      known-good rigged base to transfer weights from
  --candidate   extra weights to score (any rigged GLB of this mesh); repeatable
  --skintokens  also score SkinTokens weights for this skeleton (the ML
                candidate): runs `skintokens skin` ($SKINTOKENS_BIN, PATH, or
                ~/Games/_blender/skintokens/bin/skintokens; ~20 s on an M4).
                Never trusted blindly: scored like every other candidate
  --all-regions also change regions the check did not flag
  --motion      judge the result on this rig's clips too (as in check)
  Headline guard (default on): regions where the result tears more skin
  than the input (check's headline, on the poses and on --motion's clips)
  keep the input's weights; if the whole still tears more, the input comes
  back. --no-headline-guard turns it off (the score alone decides)
  --sheet       write an A/B compare sheet (input vs fixed)

COMMON OPTIONS:
  --poses <file.ron>  range-of-motion set (default: built-in by skeleton class)
  --no-clips          ignore the asset's own animation clips
  --voxels <N>        voxel resolution for geodesics (default 128)
  --json              machine-readable report on stdout

  -h, --help          this text;  -V, --version  print the version

ENV: WF_DEBUG=1 traces fix's region pick and band widths on stderr. SKINTOKENS_BIN: see --skintokens.
EXIT: 0 clean (fix: output passes), 1 faults found, 2 usage/IO error.
";

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

const VALUED: &[&str] = &[
    "--poses",
    "--voxels",
    "--out",
    "--cols",
    "--method",
    "--source",
    "--candidate",
    "--sheet",
    "--report",
    "--vertex-json",
    "--poses-b",
    "--rows",
    "--label-a",
    "--label-b",
    "--clips",
    "--number-from",
    "--motion",
    "--motion-clips",
    "--kind",
    "--top",
    "--seed",
    "--camera",
    "--metrics",
    "--answers",
    "--key",
];

impl Args {
    fn parse(raw: Vec<String>) -> Result<Args, String> {
        let mut pos = Vec::new();
        let mut flags = Vec::new();
        let mut it = raw.into_iter();
        while let Some(a) = it.next() {
            if a.starts_with("--") {
                if VALUED.contains(&a.as_str()) {
                    let v = it.next().ok_or_else(|| format!("{a} needs a value"))?;
                    flags.push((a, Some(v)));
                } else {
                    flags.push((a, None));
                }
            } else {
                pos.push(a);
            }
        }
        Ok(Args { pos, flags })
    }
    fn has(&self, f: &str) -> bool {
        self.flags.iter().any(|(k, _)| k == f)
    }
    fn get(&self, f: &str) -> Option<&str> {
        self.flags.iter().rev().find(|(k, _)| k == f).and_then(|(_, v)| v.as_deref())
    }
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("weights: {msg}");
    ExitCode::from(2)
}

fn ctx_opts<'a>(a: &Args, set: Option<&'a wf::poses::PoseSet>) -> Result<CtxOpts<'a>, String> {
    let mut o = CtxOpts { poses: set, clips: !a.has("--no-clips"), ..Default::default() };
    if let Some(v) = a.get("--voxels") {
        o.voxel_res = v.parse().map_err(|_| "--voxels needs an integer".to_string())?;
        if !(16..=1024).contains(&o.voxel_res) {
            return Err("--voxels must be 16..1024".into());
        }
    }
    Ok(o)
}

fn load_poses(a: &Args) -> Result<Option<wf::poses::PoseSet>, String> {
    load_poses_from(a, "--poses")
}

fn load_poses_from(a: &Args, flag: &str) -> Result<Option<wf::poses::PoseSet>, String> {
    match a.get(flag) {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            wf::poses::parse_pose_set(&text).map(Some)
        }
        None => Ok(None),
    }
}

fn cmd_check(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("check needs an input .glb") };
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let spec = wf::MotionSpec {
        source: a.get("--motion").map(PathBuf::from),
        clips: a.get("--motion-clips").map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
    };
    let checked = match wf::check_full(Path::new(input), &opts, &spec) {
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
        if let Err(e) = std::fs::write(vj, serde_json::to_vec(&doc).unwrap()) {
            return fail(format!("{vj}: {e}"));
        }
    }
    if a.has("--json") {
        println!("{}", serde_json::to_string_pretty(&rep).unwrap());
    } else {
        print!("{}", wf::report::text(&rep));
    }
    if rep.pass { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

fn fails(r: &wf::report::Report) -> usize {
    r.findings.iter().filter(|f| f.severity == "fail").count()
}

fn write_png(out: &str, img: &wf::render::Image) -> Result<(), String> {
    std::fs::write(out, img.png_bytes()).map_err(|e| format!("{out}: {e}"))
}

fn cmd_sheet(a: &Args) -> ExitCode {
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
    let (model, ctx, ev, rep) = match wf::check(Path::new(input), &opts) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let mut so = wf::render::SheetOpts::default();
    if let Some(c) = a.get("--cols") {
        match c.parse::<usize>() {
            Ok(n) if (1..=8).contains(&n) => so.cols = n,
            _ => return fail("--cols must be 1..8"),
        }
    }
    let mask = wf::report::finding_mask(&ctx, &ev, &rep);
    let info = wf::render::SheetInfo { title: &rep.file, mask: &mask, score: rep.score, tears: 0.0, pass: rep.pass, fails: fails(&rep) };
    let img = wf::render::sheet(&model, &ctx, &ev, &model.weights, &info, &so);
    if let Err(e) = write_png(out, &img) {
        return fail(e);
    }
    println!("{out}");
    if rep.pass { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

fn ffmpeg(args: &[&str]) -> bool {
    std::process::Command::new("ffmpeg").args(["-y", "-loglevel", "error"]).args(args).status().map(|s| s.success()).unwrap_or(false)
}

/// A directory of NNN.png frames into an mp4; the frames are removed once
/// it is written. False when ffmpeg is missing or fails.
fn encode_frames(frames: &Path, mp4: &Path) -> bool {
    let fps = wf::render::CLIP_FPS.to_string();
    let pattern = frames.join("%03d.png");
    let ok = ffmpeg(&[
        "-framerate",
        &fps,
        "-i",
        &pattern.to_string_lossy(),
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-crf",
        "18",
        "-movflags",
        "+faststart",
        &mp4.to_string_lossy(),
    ]);
    if ok {
        let _ = std::fs::remove_dir_all(frames);
    }
    ok
}

/// How close a rig's weights come to a pro rig's (see CLOSENESS).
fn cmd_closeness(a: &Args) -> ExitCode {
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
    let spec = wf::MotionSpec {
        source: a.get("--motion").map(PathBuf::from),
        clips: a.get("--motion-clips").map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
    };
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
        println!("{}", serde_json::to_string_pretty(&c).unwrap());
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

/// A close-up sheet per headline finding (see SHOW in the usage).
fn cmd_show(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("show needs an input .glb") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("show needs --out <dir>") };
    let top = match a.get("--top").map(|v| v.parse::<usize>()) {
        None => 6,
        Some(Ok(n)) if n >= 1 => n,
        Some(_) => return fail("--top needs a positive integer"),
    };
    let kinds: Option<Vec<String>> = a.get("--kind").map(|k| k.split(',').map(|s| s.trim().to_string()).collect());
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let spec = wf::MotionSpec {
        source: a.get("--motion").map(PathBuf::from),
        clips: a.get("--motion-clips").map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
    };
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
        if let Err(e) = write_png(&path.to_string_lossy(), &img) {
            return fail(e);
        }
        println!("{}", path.display());
        index.push(
            serde_json::json!({"file": path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), "section": sec, "finding": f}),
        );
    }
    let findings_path = dir.join("findings.json");
    let body = match serde_json::to_string_pretty(&index) {
        Ok(s) => s,
        Err(e) => return fail(format!("json: {e}")),
    };
    if let Err(e) = std::fs::write(&findings_path, body) {
        return fail(format!("{}: {e}", findings_path.display()));
    }
    ExitCode::SUCCESS
}

/// The before rig's worst failing areas on the game's clips, each beside
/// the same place on the after rig at the same frame (BEFORE-AFTER).
fn cmd_before_after(a: &Args) -> ExitCode {
    let (Some(bp), Some(ap)) = (a.pos.get(1), a.pos.get(2)) else { return fail("before-after needs <before.glb> <after.glb>") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("before-after needs --out <dir>") };
    let top = match a.get("--top").map(|v| v.parse::<usize>()) {
        None => 3,
        Some(Ok(n)) if n >= 1 => n,
        Some(_) => return fail("--top needs a positive integer"),
    };
    let kinds: Option<Vec<String>> = a.get("--kind").map(|k| k.split(',').map(|s| s.trim().to_string()).collect());
    let set = match load_poses(a) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let opts = match ctx_opts(a, set.as_ref()) {
        Ok(o) => o,
        Err(e) => return fail(e),
    };
    let spec = wf::MotionSpec {
        source: a.get("--motion").map(PathBuf::from),
        clips: a.get("--motion-clips").map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
    };
    let (b, af) = match (wf::check_full(Path::new(bp), &opts, &spec), wf::check_full(Path::new(ap), &opts, &spec)) {
        (Ok(b), Ok(a)) => (b, a),
        (Err(e), _) | (_, Err(e)) => return fail(e),
    };
    let (Some(bm), Some(am)) = (b.motion.as_ref(), af.motion.as_ref()) else {
        return fail("before-after judges the game's clips: give --motion hero.glb (or rigs with their own clips)");
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
        let p = b.model.rest[v as usize];
        (0..af.model.nverts()).min_by(|&i, &j| (af.model.rest[i] - p).len().total_cmp(&(af.model.rest[j] - p).len())).unwrap_or(0) as u32
    };
    let torn_after = wf::owner::torn(&af.model, &am.0, &af.model.weights, true);
    let area_a = wf::owner::vertex_areas(&af.model);
    let k = wf::owner::BODY_M / wf::owner::height(&af.model);
    let (pb, pa) = (wf::paint::read(&b.model), wf::paint::read(&af.model));
    let picked: Vec<&wf::owner::Finding> = b
        .rep
        .headline
        .iter()
        .filter(|s| s.name == "game clips")
        .flat_map(|s| s.findings.iter())
        .filter(|f| f.severity == "fail" && kinds.as_ref().map_or(f.kind != "limb_through", |k| k.contains(&f.kind)))
        .collect();
    let mut picked = picked;
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
        let left: f64 = after_vs.iter().filter(|&&v| torn_after[v as usize]).map(|&v| area_a[v as usize] * k * k * 1e4).sum::<f64>() + 0.0;
        let mut marked_b = vec![false; b.model.nverts()];
        for &v in &f.vertices {
            marked_b[v as usize] = true;
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
        if let Err(e) = write_png(&path.to_string_lossy(), &img) {
            return fail(e);
        }
        println!("{}  before {:.0} cm2, after {left:.0} cm2", path.display(), f.area_cm2);
        index.push(
            serde_json::json!({"file": path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(), "finding": f, "after_cm2": left}),
        );
    }
    let ba_path = dir.join("before_after.json");
    let body = match serde_json::to_string_pretty(&index) {
        Ok(s) => s,
        Err(e) => return fail(format!("json: {e}")),
    };
    if let Err(e) = std::fs::write(&ba_path, body) {
        return fail(format!("{}: {e}", ba_path.display()));
    }
    ExitCode::SUCCESS
}

fn winner_of(x: &str, y: &str, vx: f64, vy: f64, eps: f64, lower_wins: bool) -> String {
    if (vx - vy).abs() <= eps {
        "tie".into()
    } else if (vx < vy) == lower_wins {
        x.into()
    } else {
        y.into()
    }
}

/// Locate the `gk` binary via `GK_BIN`, `PATH`, sibling `_tools/gk`, or `~/.cargo/bin/gk`.
fn find_gk() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("GK_BIN")
        && !p.is_empty()
    {
        let pb = PathBuf::from(&p);
        if pb.is_file() {
            return Ok(pb);
        }
        return Err(format!("gk not found at GK_BIN={}", pb.display()));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let cand = dir.join("gk");
            if cand.is_file() {
                return Ok(cand);
            }
        }
    }
    let mut fallbacks: Vec<PathBuf> = Vec::new();
    if let Some(td) = std::env::var_os("GAMES_TOOLS_DIR").filter(|s| !s.is_empty()) {
        fallbacks.push(PathBuf::from(&td).join("gk/target/release/gk"));
        fallbacks.push(PathBuf::from(&td).join("gk/target/debug/gk"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let h = PathBuf::from(home);
        fallbacks.push(h.join("Games/_tools/gk/target/release/gk"));
        fallbacks.push(h.join("Games/_tools/gk/target/debug/gk"));
        fallbacks.push(h.join(".cargo/bin/gk"));
    }
    for cand in fallbacks {
        if cand.is_file() {
            return Ok(cand);
        }
    }
    Err("gk binary not found (install gk or set GK_BIN)".into())
}

fn cmd_err(out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let std_out = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match (err.is_empty(), std_out.is_empty()) {
        (false, false) => format!("{err} | {std_out}"),
        (false, true) => err,
        (true, false) => std_out,
        (true, true) => String::new(),
    }
}

fn format_sub_err(label: &str, out: &std::process::Output) -> String {
    let msg = cmd_err(out);
    if msg.is_empty() { format!("{label} ({})", out.status) } else { format!("{label} ({}): {msg}", out.status) }
}

fn image_rmse(a: &wf::render::Image, b: &wf::render::Image) -> f64 {
    if a.w != b.w || a.h != b.h || a.rgb.len() != b.rgb.len() || a.rgb.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = a
        .rgb
        .iter()
        .zip(&b.rgb)
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum();
    (sum_sq / a.rgb.len() as f64).sqrt()
}

/// Same-frame blind pairs per claim at the game camera (delegated to `gk blind make`).
fn cmd_blind(a: &Args) -> ExitCode {
    let (Some(pa), Some(pb)) = (a.pos.get(1), a.pos.get(2)) else { return fail("blind needs two .glb files") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("blind needs --out <dir>") };
    let top = match a.get("--top").map(|v| v.parse::<usize>()) {
        None => 6,
        Some(Ok(n)) if n >= 1 => n,
        Some(_) => return fail("--top needs a positive integer"),
    };
    let seed: u64 = match a.get("--seed").map(|v| v.parse()) {
        None => 20261115,
        Some(Ok(n)) => n,
        Some(_) => return fail("--seed needs an integer"),
    };
    let kinds: Option<Vec<String>> = a.get("--kind").map(|k| k.split(',').map(|s| s.trim().to_string()).collect());
    let camera = match a.get("--camera").map(wf::blind::Camera::parse) {
        None => wf::blind::Camera::Follow,
        Some(Some(c)) => c,
        Some(None) => return fail("--camera is follow or talk"),
    };
    // The follow camera's lens and distance, from --metrics or $METRICS_FILE / $HLH_METRICS.
    let share = if camera == wf::blind::Camera::Follow {
        let res = match a.get("--metrics") {
            Some(p) => wf::blind::game_frame_share_from_path(Path::new(p)),
            None => wf::blind::game_frame_share(),
        };
        match res {
            Ok(s) => s,
            Err(e) => return fail(e),
        }
    } else {
        0.0
    };
    let gk_bin = match find_gk() {
        Ok(b) => b,
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
    let spec = wf::MotionSpec {
        source: a.get("--motion").map(PathBuf::from),
        clips: a.get("--motion-clips").map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
    };
    let (ca, cb) = match (wf::check_full(Path::new(pa), &opts, &spec), wf::check_full(Path::new(pb), &opts, &spec)) {
        (Ok(x), Ok(y)) => (x, y),
        (Err(e), _) | (_, Err(e)) => return fail(e),
    };
    let use_clips = match (&ca.motion, &cb.motion) {
        (Some(x), Some(y)) => x.0.poses.len() == y.0.poses.len() && !x.0.poses.is_empty(),
        _ => false,
    };
    let sec_name = if use_clips { "game clips" } else { "poses" };
    let (Some(seca), Some(secb)) =
        (ca.rep.headline.iter().find(|s| s.name == sec_name), cb.rep.headline.iter().find(|s| s.name == sec_name))
    else {
        return fail("blind: a rig has no headline section");
    };
    let torn_of = |sec: &wf::owner::Section, n: usize| -> Vec<bool> {
        let mut t = vec![false; n];
        for f in sec.findings.iter().filter(|f| f.severity == "fail" && f.kind != "limb_through") {
            for &v in &f.vertices {
                t[v as usize] = true;
            }
        }
        t
    };
    let (ctxa, ctxb) = match (use_clips, ca.motion.as_ref(), cb.motion.as_ref()) {
        (true, Some((a, _, _)), Some((b, _, _))) => (a, b),
        _ => (&ca.ctx, &cb.ctx),
    };
    let (torna, tornb) = (torn_of(seca, ca.model.nverts()), torn_of(secb, cb.model.nverts()));
    let (areaa, areab) = (wf::owner::vertex_areas(&ca.model), wf::owner::vertex_areas(&cb.model));
    let ka = wf::owner::BODY_M / wf::owner::height(&ca.model);
    let kb = wf::owner::BODY_M / wf::owner::height(&cb.model);

    struct Claim {
        kind: String,
        part: String,
        pose: usize,
        frame: String,
        src: usize,
        verts: Vec<u32>,
        area: f64,
    }
    let mut claims: Vec<Claim> = Vec::new();
    let through = a.has("--through");
    for (src, sec, other_ctx) in [(0usize, seca, ctxb), (1usize, secb, ctxa)] {
        for f in sec.findings.iter().filter(|f| f.severity == "fail" && (through || f.kind != "limb_through")) {
            if kinds.as_ref().is_some_and(|k| !k.contains(&f.kind)) || f.worst_pose >= other_ctx.poses.len() {
                continue;
            }
            let c = Claim {
                kind: f.kind.clone(),
                part: f.part.clone(),
                pose: f.worst_pose,
                frame: f.worst.clone(),
                src,
                verts: f.vertices.clone(),
                area: f.area_cm2,
            };
            match claims.iter_mut().find(|x| x.kind == c.kind && x.part == c.part) {
                Some(x) if c.area > x.area => *x = c,
                Some(_) => {}
                None => claims.push(c),
            }
        }
    }
    claims.sort_by(|x, y| y.area.total_cmp(&x.area));
    let mut out_of_frame: Vec<String> = Vec::new();
    if camera == wf::blind::Camera::Talk {
        claims.retain(|c| {
            let rest = if c.src == 0 { &ca.model.rest } else { &cb.model.rest };
            let keep = wf::blind::in_talk_frame(rest, &c.verts);
            if !keep {
                out_of_frame.push(format!("{}: {}", c.kind.replace('_', " "), c.part));
            }
            keep
        });
    }
    claims.truncate(top);

    let planted_cands = wf::blind::planted_candidates(&ca.model, ctxa, use_clips);
    let Some(primary_planted) = planted_cands.first() else {
        return fail("blind: rig A has no mirrored limb pair to plant a bad case from");
    };
    let (paint_a, paint_b) = (wf::paint::read(&ca.model), wf::paint::read(&cb.model));
    let torn_cm2 = |verts_src: usize, verts: &[u32], rig: usize| -> f64 {
        let (model_s, model_d, torn, area, k) = if rig == 0 {
            (if verts_src == 0 { &ca.model } else { &cb.model }, &ca.model, &torna, &areaa, ka)
        } else {
            (if verts_src == 0 { &ca.model } else { &cb.model }, &cb.model, &tornb, &areab, kb)
        };
        wf::blind::map_vertices(model_s, model_d, verts)
            .iter()
            .filter(|&&v| torn[v as usize])
            .map(|&v| area[v as usize] * k * k * 1e4)
            .sum::<f64>()
            + 0.0
    };

    struct Pending {
        stem: String,
        category: &'static str,
        focus: String,
        frame: String,
        pose: usize,
        view_src: usize,
        marked: Vec<u32>,
        tears_cm2: std::collections::BTreeMap<String, f64>,
        tears_winner: String,
        score_winner: String,
        expected: String,
        limg: wf::render::Image,
        rimg: wf::render::Image,
        pimg: wf::render::Image,
        planted_rmse: f64,
    }

    let sa = wf::blind::BlindSide { model: &ca.model, ctx: ctxa, weights: &ca.model.weights, paint: &paint_a };
    let sb = wf::blind::BlindSide { model: &cb.model, ctx: ctxb, weights: &cb.model.weights, paint: &paint_b };

    let render_triple = |planted_w: &wf::scene::Weights,
                         pose: usize,
                         view_src: usize,
                         marked_idxs: &[u32]|
     -> (wf::render::Image, wf::render::Image, wf::render::Image, f64) {
        let sp = wf::blind::BlindSide { model: &ca.model, ctx: ctxa, weights: planted_w, paint: &paint_a };
        let vs = if view_src == 1 {
            &sb
        } else if view_src == 2 {
            &sp
        } else {
            &sa
        };
        let src_model = if view_src == 1 { &cb.model } else { &ca.model };
        let mut marked = vec![false; vs.model.nverts()];
        for &v in wf::blind::map_vertices(src_model, vs.model, marked_idxs).iter() {
            marked[v as usize] = true;
        }
        let posed = |s: &wf::blind::BlindSide<'_>| wf::skin::lbs(s.model, &s.ctx.mats[pose.min(s.ctx.mats.len() - 1)], s.weights);
        let (pa_p, pb_p) = (posed(&sa), posed(&sb));
        let (view, fit) = match camera {
            wf::blind::Camera::Follow => {
                let view = wf::blind::claim_view(vs, pose, &marked);
                let fit = wf::blind::game_fit(&view, &[&pa_p, &pb_p], wf::owner::height(sa.model), 520, 640, share);
                (view, fit)
            }
            wf::blind::Camera::Talk => {
                let rest = &vs.model.rest;
                let mid = rest.iter().map(|q| q.x).sum::<f64>() / rest.len().max(1) as f64;
                let picked: Vec<usize> = marked.iter().enumerate().filter(|(_, m)| **m).map(|(i, _)| i).collect();
                let side_x =
                    if picked.is_empty() { 1.0 } else { picked.iter().map(|&i| rest[i].x).sum::<f64>() / picked.len() as f64 - mid };
                let view = wf::blind::talk_view(side_x);
                let fit = wf::blind::talk_fit(&view, &ca.model.rest, &pa_p, wf::owner::height(&ca.model), 640);
                (view, fit)
            }
        };
        let img_a = wf::blind::cell_image(&sa, pose, &view, fit, 520, 640, "");
        let img_b = wf::blind::cell_image(&sb, pose, &view, fit, 520, 640, "");
        let img_p = wf::blind::cell_image(&sp, pose, &view, fit, 520, 640, "");
        let rmse = image_rmse(&img_a, &img_p);
        (img_a, img_b, img_p, rmse)
    };

    let mut claim_items: Vec<Pending> = Vec::new();
    let score_w = winner_of("A", "B", ca.rep.score, cb.rep.score, 1.0, false);
    for (idx, c) in claims.iter().enumerate() {
        let (ta, tb) = (torn_cm2(c.src, &c.verts, 0), torn_cm2(c.src, &c.verts, 1));
        let tw = winner_of("A", "B", ta, tb, 0.5, true);
        let (limg, rimg, pimg, planted_rmse) = render_triple(&primary_planted.weights, c.pose, c.src, &c.verts);
        let safe: String = format!("{}_{}", c.kind, c.part).chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect();
        claim_items.push(Pending {
            stem: format!("{:03}_{safe}", idx + 1),
            category: "claim",
            focus: format!("{}: {}", c.kind.replace('_', " "), c.part),
            frame: c.frame.clone(),
            pose: c.pose,
            view_src: c.src,
            marked: c.verts.clone(),
            tears_cm2: [("A".to_string(), (ta * 10.0).round() / 10.0), ("B".to_string(), (tb * 10.0).round() / 10.0)].into_iter().collect(),
            tears_winner: tw.clone(),
            score_winner: score_w.clone(),
            expected: tw,
            limg,
            rimg,
            pimg,
            planted_rmse,
        });
    }
    if camera == wf::blind::Camera::Talk {
        let (limg, rimg, pimg, planted_rmse) = render_triple(&primary_planted.weights, 0, 0, &[]);
        claim_items.push(Pending {
            stem: "999_look_face".into(),
            category: "look",
            focus: "the face and head at the talk camera".into(),
            frame: ctxa.poses[0].name.clone(),
            pose: 0,
            view_src: 0,
            marked: Vec::new(),
            tears_cm2: std::collections::BTreeMap::new(),
            tears_winner: "tie".into(),
            score_winner: score_w.clone(),
            expected: "tie".into(),
            limg,
            rimg,
            pimg,
            planted_rmse,
        });
    }

    let upright: Vec<bool> = ctxa.mats.iter().map(|m| wf::blind::is_upright_pose(&ca.model, m)).collect();
    let any_upright = upright.iter().any(|&u| u);

    let stage = std::env::temp_dir().join(format!("weights_gk_blind_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    let stage_a = stage.join("a");
    let stage_b = stage.join("b");
    let stage_out = stage.join("out");
    if let Err(e) = std::fs::create_dir_all(&stage_a).and_then(|_| std::fs::create_dir_all(&stage_b)) {
        return fail(format!("{}: {e}", stage.display()));
    }
    let cleanup = |s: &Path| {
        let _ = std::fs::remove_dir_all(s);
    };

    // Ensure stage/a's first frame (paired by `gk blind make` with `--planted`) is upright when possible and visibly shows `PLANTED_A`.
    let best_claim_idx = claim_items
        .iter()
        .enumerate()
        .filter(|(_, c)| (!any_upright || upright.get(c.pose).copied().unwrap_or(false)) && c.planted_rmse > 8.0)
        .max_by(|(_, x), (_, y)| x.planted_rmse.total_cmp(&y.planted_rmse))
        .map(|(i, _)| i);

    let stage_planted = stage.join("planted.png");
    let (chosen_planted, planted_frame): (&wf::blind::PlantedCandidate, String) = if let Some(idx) = best_claim_idx {
        claim_items[idx].stem = format!("000_{}", &claim_items[idx].stem[4..]);
        if let Err(e) = write_png(&stage_planted.to_string_lossy(), &claim_items[idx].pimg) {
            cleanup(&stage);
            return fail(e);
        }
        (primary_planted, claim_items[idx].frame.clone())
    } else {
        let ta_all: f64 = (0..ca.model.nverts()).filter(|&v| torna[v]).map(|v| areaa[v] * ka * ka * 1e4).sum();
        let tb_all: f64 = (0..cb.model.nverts()).filter(|&v| tornb[v]).map(|v| areab[v] * kb * kb * 1e4).sum();
        let tw_all = winner_of("A", "B", ta_all, tb_all, 0.5, true);

        let rest = &ca.model.rest;
        let floor = rest.iter().map(|q| q.y).fold(f64::MAX, f64::min);
        let h = wf::owner::height(&ca.model).max(1e-9);
        let (talk_lo, _) = wf::blind::talk_frame_span();
        let talk_verts: Vec<usize> = (0..rest.len()).filter(|&i| (rest[i].y - floor) / h >= talk_lo).collect();

        let mut best_pick: Option<(&wf::blind::PlantedCandidate, usize, wf::render::Image, wf::render::Image, wf::render::Image, f64)> =
            None;
        for cand in &planted_cands {
            let focus_verts: Vec<usize> = match camera {
                wf::blind::Camera::Talk => talk_verts.clone(),
                wf::blind::Camera::Follow if !cand.vertices.is_empty() => cand.vertices.iter().map(|&v| v as usize).collect(),
                wf::blind::Camera::Follow => (0..rest.len()).collect(),
            };
            let mut ranked_poses: Vec<(usize, f64)> = (0..ctxa.mats.len())
                .filter(|&i| !any_upright || upright[i])
                .map(|i| {
                    let mats = &ctxa.mats[i];
                    let d: f64 = focus_verts
                        .iter()
                        .map(|&v| {
                            let pa = wf::skin::lbs_vertex(mats, &ca.model.weights[v], rest[v]);
                            let pp = wf::skin::lbs_vertex(mats, &cand.weights[v], rest[v]);
                            (pp - pa).len()
                        })
                        .sum();
                    (i, d)
                })
                .collect();
            ranked_poses.sort_by(|a, b| b.1.total_cmp(&a.1));
            let mut try_poses: Vec<usize> = ranked_poses.into_iter().take(6).map(|(p, _)| p).collect();
            if (!any_upright || upright.get(cand.pose).copied().unwrap_or(false)) && !try_poses.contains(&cand.pose) {
                try_poses.push(cand.pose);
            }
            for pose in try_poses {
                let (limg, rimg, pimg, rmse) = render_triple(&cand.weights, pose, 2, &cand.vertices);
                if best_pick.as_ref().is_none_or(|b| rmse > b.5) {
                    best_pick = Some((cand, pose, limg, rimg, pimg, rmse));
                }
            }
            if best_pick.as_ref().is_some_and(|b| b.5 >= 12.0) {
                break;
            }
        }
        let Some((cand, pose, limg, rimg, pimg, planted_rmse)) = best_pick else {
            cleanup(&stage);
            return fail("blind: no pose available to render planted bad case");
        };
        if let Err(e) = write_png(&stage_planted.to_string_lossy(), &pimg) {
            cleanup(&stage);
            return fail(e);
        }
        let frame_name = ctxa.poses[pose].name.clone();
        claim_items.insert(
            0,
            Pending {
                stem: "000_whole_body".into(),
                category: "claim",
                focus: "the whole body at the game camera".into(),
                frame: frame_name.clone(),
                pose,
                view_src: 2,
                marked: cand.vertices.clone(),
                tears_cm2: [("A".to_string(), (ta_all * 10.0).round() / 10.0), ("B".to_string(), (tb_all * 10.0).round() / 10.0)]
                    .into_iter()
                    .collect(),
                tears_winner: tw_all.clone(),
                score_winner: score_w.clone(),
                expected: tw_all,
                limg,
                rimg,
                pimg,
                planted_rmse,
            },
        );
        (cand, frame_name)
    };
    for c in &claim_items {
        let _ = (c.pose, c.view_src, c.marked.len(), c.planted_rmse);
    }

    for c in &claim_items {
        if let Err(e) = write_png(&stage_a.join(format!("{}.png", c.stem)).to_string_lossy(), &c.limg)
            .and_then(|_| write_png(&stage_b.join(format!("{}.png", c.stem)).to_string_lossy(), &c.rimg))
        {
            cleanup(&stage);
            return fail(e);
        }
    }

    let question = "Which side's skin bends and follows cleaner in this frame? Answer left, right, or cant_tell.";
    let gk_flag = |f: &str| ["--", f].concat();
    let gk_res = std::process::Command::new(&gk_bin)
        .arg("blind")
        .arg("make")
        .arg(gk_flag("a"))
        .arg(&stage_a)
        .arg(gk_flag("b"))
        .arg(&stage_b)
        .arg(gk_flag("planted"))
        .arg(&stage_planted)
        .arg("--out")
        .arg(&stage_out)
        .arg("--seed")
        .arg(seed.to_string())
        .arg(gk_flag("question"))
        .arg(question)
        .output();
    let gk_out = match gk_res {
        Ok(o) => o,
        Err(e) => {
            cleanup(&stage);
            return fail(format!("{}: {e}", gk_bin.display()));
        }
    };
    if !gk_out.status.success() {
        cleanup(&stage);
        return fail(format_sub_err("gk blind make failed", &gk_out));
    }

    let raw_key_path = stage.join("out.key.json");
    let mut kdoc: serde_json::Value =
        match std::fs::read(&raw_key_path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => {
                cleanup(&stage);
                return fail(format!("{}: {e}", raw_key_path.display()));
            }
        };

    let planted_ev = wf::metrics::evaluate_with(&ca.model, ctxa, &chosen_planted.weights, false);
    let planted_score = wf::report::build("planted", &ca.model, ctxa, &planted_ev).score;
    let planted_torn = wf::owner::torn(&ca.model, ctxa, &chosen_planted.weights, use_clips);
    let pta: f64 = chosen_planted.vertices.iter().filter(|&&v| torna[v as usize]).map(|&v| areaa[v as usize] * ka * ka * 1e4).sum();
    let ptp: f64 = chosen_planted.vertices.iter().filter(|&&v| planted_torn[v as usize]).map(|&v| areaa[v as usize] * ka * ka * 1e4).sum();
    let planted_tears_winner = winner_of("A", "PLANTED_A", pta, ptp, 0.5, true);
    let planted_score_winner = winner_of("A", "PLANTED_A", ca.rep.score, planted_score, 1.0, false);

    let c0 = &claim_items[0];
    let c_last = claim_items.iter().max_by(|x, y| x.stem.cmp(&y.stem)).unwrap_or(c0);
    if let Some(pairs_arr) = kdoc.get_mut("pairs").and_then(|v| v.as_array_mut()) {
        for (idx, pval) in pairs_arr.iter_mut().enumerate() {
            let pair_num = idx + 1;
            let kind = pval.get("kind").and_then(|v| v.as_str()).unwrap_or("claim").to_string();
            let left_tag = pval.get("left").and_then(|v| v.as_str()).unwrap_or("a").to_string();
            let stem = pval.get("claim").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let Some(pobj) = pval.as_object_mut() else {
                cleanup(&stage);
                return fail("internal error: blind pair metadata is not an object".to_string());
            };
            pobj.insert("pair".into(), serde_json::json!(pair_num));
            match kind.as_str() {
                "control" => {
                    pobj.insert("category".into(), serde_json::json!("control"));
                    pobj.insert("focus".into(), serde_json::json!("the whole body at the game camera"));
                    pobj.insert("frame".into(), serde_json::json!(c_last.frame));
                    pobj.insert("left_rig".into(), serde_json::json!("A"));
                    pobj.insert("right_rig".into(), serde_json::json!("A"));
                    pobj.insert("expected".into(), serde_json::json!("cant_tell"));
                    pobj.insert("tears_cm2".into(), serde_json::json!({"A": 0.0}));
                    pobj.insert("tears_winner".into(), serde_json::json!("tie"));
                    pobj.insert("score_winner".into(), serde_json::json!("tie"));
                }
                "planted" => {
                    let (lr, rr) = if left_tag == "a" { ("A", "PLANTED_A") } else { ("PLANTED_A", "A") };
                    pobj.insert("category".into(), serde_json::json!("planted"));
                    pobj.insert("focus".into(), serde_json::json!("the whole body at the game camera"));
                    pobj.insert("frame".into(), serde_json::json!(planted_frame));
                    pobj.insert("left_rig".into(), serde_json::json!(lr));
                    pobj.insert("right_rig".into(), serde_json::json!(rr));
                    pobj.insert("expected".into(), serde_json::json!("A"));
                    pobj.insert(
                        "tears_cm2".into(),
                        serde_json::json!({"A": (pta * 10.0).round() / 10.0 + 0.0, "PLANTED_A": (ptp * 10.0).round() / 10.0 + 0.0}),
                    );
                    pobj.insert("tears_winner".into(), serde_json::json!(planted_tears_winner));
                    pobj.insert("score_winner".into(), serde_json::json!(planted_score_winner));
                }
                _ => {
                    let cm = claim_items.iter().find(|c| c.stem == stem).unwrap_or(c0);
                    let (lr, rr) = if left_tag == "a" { ("A", "B") } else { ("B", "A") };
                    pobj.insert("category".into(), serde_json::json!(cm.category));
                    pobj.insert("focus".into(), serde_json::json!(cm.focus));
                    pobj.insert("frame".into(), serde_json::json!(cm.frame));
                    pobj.insert("left_rig".into(), serde_json::json!(lr));
                    pobj.insert("right_rig".into(), serde_json::json!(rr));
                    pobj.insert("expected".into(), serde_json::json!(cm.expected));
                    pobj.insert("tears_cm2".into(), serde_json::json!(cm.tears_cm2));
                    pobj.insert("tears_winner".into(), serde_json::json!(cm.tears_winner));
                    pobj.insert("score_winner".into(), serde_json::json!(cm.score_winner));
                }
            }
        }
    }
    if let Some(kobj) = kdoc.as_object_mut() {
        kobj.insert("rig_a".into(), serde_json::json!(ca.rep.file));
        kobj.insert("rig_b".into(), serde_json::json!(cb.rep.file));
        kobj.insert("section".into(), serde_json::json!(sec_name));
        kobj.insert("planted".into(), serde_json::json!(chosen_planted.what));
        kobj.insert("tears_pct".into(), serde_json::json!({"A": seca.tears_pct, "B": secb.tears_pct}));
        kobj.insert(
            "scores".into(),
            serde_json::json!({"A": ca.rep.score, "B": cb.rep.score, "PLANTED_A": (planted_score * 10.0).round() / 10.0}),
        );
        kobj.insert("camera".into(), serde_json::json!(camera.name()));
        kobj.insert("out_of_frame".into(), serde_json::json!(out_of_frame));
    }

    if let Err(e) = std::fs::create_dir_all(dir) {
        cleanup(&stage);
        return fail(format!("{}: {e}", dir.display()));
    }
    match std::fs::read_dir(&stage_out) {
        Ok(entries) => {
            for ent in entries.flatten() {
                let fname = ent.file_name();
                let dst = dir.join(&fname);
                if let Err(e) = std::fs::copy(ent.path(), &dst) {
                    cleanup(&stage);
                    return fail(format!("{}: {e}", dst.display()));
                }
                if fname.to_string_lossy().starts_with("pair-")
                    && fname.to_string_lossy().ends_with(".png")
                    && !fname.to_string_lossy().contains("-left")
                    && !fname.to_string_lossy().contains("-right")
                {
                    println!("{}", dst.display());
                }
            }
        }
        Err(e) => {
            cleanup(&stage);
            return fail(format!("{}: {e}", stage_out.display()));
        }
    }

    let key_path = match a.get("--key") {
        Some(k) => PathBuf::from(k),
        None => {
            let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "blind".into());
            dir.parent().unwrap_or(Path::new(".")).join(format!("{name}.key.json"))
        }
    };
    if let Err(e) = std::fs::write(&key_path, serde_json::to_string_pretty(&kdoc).unwrap()) {
        cleanup(&stage);
        return fail(format!("{}: {e}", key_path.display()));
    }
    cleanup(&stage);

    println!(
        "blind ({} camera): {} pairs ({} claims, a can't-tell control, a planted bad case) on the {sec_name}; A torn {:.2}%, B torn {:.2}%; key {}",
        camera.name(),
        claim_items.len() + 2,
        claims.len(),
        seca.tears_pct,
        secb.tears_pct,
        key_path.display()
    );
    ExitCode::SUCCESS
}

/// Lays a judge's answers against tears and the old score (delegates blind check to `gk blind score`).
fn cmd_blind_score(a: &Args) -> ExitCode {
    let Some(dir) = a.pos.get(1).map(Path::new) else { return fail("blind-score needs a blind <dir>") };
    let Some(answers_path) = a.get("--answers") else { return fail("blind-score needs --answers answers.json") };
    let key_path = match a.get("--key") {
        Some(k) => PathBuf::from(k),
        None => {
            let name = dir.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "blind".into());
            let sibling = dir.parent().unwrap_or(Path::new(".")).join(format!("{name}.key.json"));
            if sibling.exists() { sibling } else { dir.join("answer_key.json") }
        }
    };
    let key_bytes = match std::fs::read(&key_path) {
        Ok(b) => b,
        Err(e) => return fail(format!("{}: {e}", key_path.display())),
    };
    let key_val: serde_json::Value = match serde_json::from_slice(&key_bytes) {
        Ok(v) => v,
        Err(e) => return fail(format!("{}: {e}", key_path.display())),
    };
    let key: wf::blind::BlindKey = match serde_json::from_value(key_val.clone()) {
        Ok(k) => k,
        Err(e) => return fail(format!("{}: {e}", key_path.display())),
    };
    let raw: serde_json::Value =
        match std::fs::read(answers_path).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => return fail(format!("{answers_path}: {e}")),
        };
    let obj = raw.get("answers").unwrap_or(&raw);
    let Some(obj) = obj.as_object() else { return fail("answers must be a JSON object of pair -> left/right/cant_tell") };
    let mut answers: std::collections::BTreeMap<usize, String> = std::collections::BTreeMap::new();
    let mut norm_by_id = serde_json::Map::new();
    for (k, v) in obj {
        let digits: String = k.chars().filter(|c| c.is_ascii_digit()).collect();
        if let (Ok(n), Some(s)) = (digits.parse::<usize>(), v.as_str()) {
            answers.insert(n, s.to_string());
            let norm: String = s.trim().to_lowercase().chars().filter(|c| !matches!(c, '\'' | ' ' | '-' | '_')).collect();
            let canon = if ["canttell", "cant", "neither", "same", "equal"].contains(&norm.as_str()) { "cant_tell" } else { norm.as_str() };
            norm_by_id.insert(format!("pair-{n:02}"), serde_json::json!(canon));
        }
    }

    let is_gk_key = key_val.get("version").and_then(|v| v.as_i64()) == Some(1);
    let gk_verdict: Option<serde_json::Value> = if is_gk_key {
        let gk_bin = match find_gk() {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        let tmp_ans = std::env::temp_dir().join(format!("weights_gk_ans_{}.json", std::process::id()));
        if let Err(e) = std::fs::write(&tmp_ans, serde_json::to_string(&serde_json::Value::Object(norm_by_id)).unwrap()) {
            return fail(format!("{}: {e}", tmp_ans.display()));
        }
        let gk_res = std::process::Command::new(&gk_bin)
            .arg("blind")
            .arg("score")
            .arg(dir)
            .arg("--answers")
            .arg(&tmp_ans)
            .arg("--key")
            .arg(&key_path)
            .arg("--json")
            .output();
        let _ = std::fs::remove_file(&tmp_ans);
        let gk_out = match gk_res {
            Ok(o) => o,
            Err(e) => return fail(format!("{}: {e}", gk_bin.display())),
        };
        if !matches!(gk_out.status.code(), Some(0 | 1)) {
            return fail(format_sub_err("gk blind score failed", &gk_out));
        }
        match serde_json::from_slice(&gk_out.stdout) {
            Ok(v) => Some(v),
            Err(e) => return fail(format!("gk blind score returned invalid JSON: {e}")),
        }
    } else {
        None
    };

    let mut v = wf::blind::score_pairs(&key, &answers);
    if let Some(ref gkv) = gk_verdict {
        if let Some(ctrl) = gkv.get("control_ok").and_then(|x| x.as_bool()) {
            v.control_ok = v.control_ok && ctrl;
        }
        if let Some(pl) = gkv.get("planted_ok").and_then(|x| x.as_bool()) {
            v.planted_ok = v.planted_ok && pl;
        }
        v.pass = v.control_ok && v.planted_ok && v.planted_tears_ok && gkv.get("pass").and_then(|x| x.as_bool()).unwrap_or(true);
    }

    for p in &v.pairs {
        println!("pair {:02} [{}] {}: {}", p.pair, p.category, p.focus, p.note);
    }
    println!(
        "judge agrees with tears on {}/{} claims; the old score agrees with the judge on {}/{}; retired on pairs {:?}",
        v.judge_agrees_tears, v.claims_judged, v.score_agrees_judge, v.claims_judged, v.score_retired_pairs
    );
    println!(
        "control {}; planted {}",
        if v.control_ok { "ok" } else { "FAILED" },
        if v.planted_ok && v.planted_tears_ok { "ok" } else { "FAILED" }
    );
    let out = dir.join("verdict.json");
    if let Err(e) = std::fs::write(&out, serde_json::to_string_pretty(&v).unwrap()) {
        return fail(format!("{}: {e}", out.display()));
    }
    println!("{}", out.display());
    if v.pass { ExitCode::SUCCESS } else { ExitCode::from(1) }
}

/// Loops of one rig for a person to judge (see PROBE in the usage).
fn cmd_probe(a: &Args) -> ExitCode {
    let Some(input) = a.pos.get(1) else { return fail("probe needs an input .glb") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("probe needs --out <dir>") };
    let first = match a.get("--number-from").map(|v| v.parse::<usize>()) {
        None => 1,
        Some(Ok(n)) => n,
        Some(Err(_)) => return fail("--number-from needs an integer"),
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
            let res = std::fs::create_dir_all(&frames).and_then(|_| std::fs::write(frames.join(format!("{f:03}.png")), img.png_bytes()));
            if let Err(e) = res {
                err.get_or_insert(format!("{}: {e}", frames.display()));
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
        let res = std::fs::create_dir_all(&d).and_then(|_| std::fs::write(d.join(format!("{f:03}.png")), img.png_bytes()));
        if let Err(e) = res {
            err.get_or_insert(io(e));
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

fn cmd_compare(a: &Args) -> ExitCode {
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
    let rows = match a.get("--rows").map(|v| v.parse::<usize>()) {
        None => 5,
        Some(Ok(n)) if (1..=12).contains(&n) => n,
        Some(_) => return fail("--rows must be 1..12"),
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
    if let Err(e) = write_png(out, &img) {
        return fail(e);
    }
    println!("{out}");
    ExitCode::SUCCESS
}

/// The ML candidate: SkinTokens weights for the input's own skeleton
/// (`skintokens skin`, sibling repo ~/Games/_blender/skintokens). Err = the tool
/// is missing (usage error); Ok(None) = it ran and failed, so the fix goes on
/// without it (a warning, never a worse result).
fn skintokens_candidate(input: &str) -> Result<Option<wf::scene::Model>, String> {
    let bin = std::env::var_os("SKINTOKENS_BIN")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("PATH").and_then(|p| std::env::split_paths(&p).map(|d| d.join("skintokens")).find(|f| f.is_file())))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Games/_blender/skintokens/bin/skintokens")).filter(|f| f.is_file())
        })
        .ok_or("skintokens not found (set SKINTOKENS_BIN or see ~/Games/_blender/skintokens)")?;
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

fn cmd_fix(a: &Args) -> ExitCode {
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
    // A body whose fused legs were cut apart (meshkit unfuse records the
    // cut): the repair starts from each half following its own leg.
    // The writer keeps the file's own bytes for every vertex whose weights
    // equal the loaded ones, so the file is written from an untouched
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
    let cand_paths: Vec<&str> = a.flags.iter().filter(|(k, _)| k == "--candidate").map(|(_, v)| v.as_deref().unwrap_or("")).collect();
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
    // The game's clips, for the headline guard.
    let spec = wf::MotionSpec {
        source: a.get("--motion").map(PathBuf::from),
        clips: a.get("--motion-clips").map(|c| c.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
    };
    let motion_ctx = match wf::motion_for(&model, false, &spec) {
        Ok((Some(m), _)) => Some(ctx.with_poses(&model, m.poses)),
        Ok((None, notes)) => {
            for n in notes {
                eprintln!("weights: {n}");
            }
            None
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
    if let Err(e) = std::fs::write(out, glb.to_bytes()) {
        return fail(format!("{out}: {e}"));
    }
    let report_path = a.get("--report").map(str::to_string).unwrap_or_else(|| {
        let p = Path::new(out);
        p.with_file_name(format!("{}.report.json", p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()))
            .to_string_lossy()
            .into_owned()
    });
    if let Err(e) = std::fs::write(&report_path, serde_json::to_string_pretty(&res.fix).unwrap()) {
        return fail(format!("{report_path}: {e}"));
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
        if let Err(e) = write_png(sheet_path, &wf::render::compare(&sa, &sb, &wf::render::SheetOpts::default())) {
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

fn cmd_dump(a: &Args) -> ExitCode {
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
    if let Err(e) = std::fs::write(out, serde_json::to_vec(&doc).unwrap()) {
        return fail(format!("{out}: {e}"));
    }
    println!("{out}");
    ExitCode::SUCCESS
}

fn cmd_fixture(a: &Args) -> ExitCode {
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
        if let Err(e) = std::fs::write(&path, glb.to_bytes()) {
            return fail(format!("{}: {e}", path.display()));
        }
        println!("{}", path.display());
    }
    ExitCode::SUCCESS
}

/// The flags each command takes; anything else is a usage error (a typo
/// such as `--jsn` used to be ignored without a word).
fn flags_of(cmd: &str) -> Option<Vec<&'static str>> {
    const CTX: [&str; 3] = ["--poses", "--no-clips", "--voxels"];
    let own: &[&str] = match cmd {
        "check" => &["--json", "--vertex-json", "--motion", "--motion-clips"],
        "sheet" => &["--out", "--cols"],
        "probe" => &["--out", "--number-from"],
        "blind" => &["--out", "--motion", "--motion-clips", "--kind", "--top", "--seed", "--key", "--camera", "--metrics", "--through"],
        "blind-score" => &["--answers", "--key"],
        "show" => &["--out", "--motion", "--motion-clips", "--kind", "--top"],
        "before-after" => &["--out", "--motion", "--motion-clips", "--kind", "--top"],
        "closeness" => &["--json", "--motion", "--motion-clips"],
        "compare" => &["--out", "--poses-b", "--rows", "--label-a", "--label-b", "--zoom", "--clips"],
        "fix" => &[
            "--out",
            "--method",
            "--source",
            "--candidate",
            "--skintokens",
            "--all-regions",
            "--report",
            "--sheet",
            "--motion",
            "--motion-clips",
            "--no-headline-guard",
        ],
        "dump" | "fixture" => return Some(vec!["--out"]),
        _ => return None,
    };
    Some(CTX.iter().chain(own).copied().collect())
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if raw.is_empty() || raw.iter().any(|a| a == "-h" || a == "--help") {
        print!("{HELP}");
        return if raw.is_empty() { ExitCode::from(2) } else { ExitCode::SUCCESS };
    }
    if raw.iter().any(|a| a == "-V" || a == "--version") {
        println!("weights {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    let a = match Args::parse(raw) {
        Ok(a) => a,
        Err(e) => return fail(e),
    };
    let Some(cmd) = a.pos.first() else { return fail("no command (try --help)") };
    if let Some(allowed) = flags_of(cmd)
        && let Some((f, _)) = a.flags.iter().find(|(f, _)| !allowed.contains(&f.as_str()))
    {
        return fail(format!("unknown flag {f} for {cmd} (try --help)"));
    }
    match a.pos[0].as_str() {
        "check" => cmd_check(&a),
        "fix" => cmd_fix(&a),
        "sheet" => cmd_sheet(&a),
        "compare" => cmd_compare(&a),
        "blind" => cmd_blind(&a),
        "blind-score" => cmd_blind_score(&a),
        "probe" => cmd_probe(&a),
        "show" => cmd_show(&a),
        "before-after" => cmd_before_after(&a),
        "closeness" => cmd_closeness(&a),
        "dump" => cmd_dump(&a),
        "fixture" => cmd_fixture(&a),
        other => fail(format!("unknown command {other} (try --help)")),
    }
}
