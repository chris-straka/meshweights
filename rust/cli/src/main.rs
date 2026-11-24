//! `weights`: find and fix bad skin weights on rigged GLB characters.
//! Exit codes: 0 = clean / success, 1 = deformation faults found,
//! 2 = usage or IO error (same shape as rfcheck).

mod args;
mod cmd;
mod common;

use std::process::ExitCode;

use args::Args;
use common::fail;

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
                  follow camera's lens and distance from --metrics or $METRICS_FILE;
                  --through: limbs passing through each other are claims too)
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
  --motion <glb>   play this rig's clips on the checked rig,
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
  `~/.cargo/bin/gk`).
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
                candidate): runs `skintokens skin` ($SKINTOKENS_BIN or PATH;
                ~20 s on an Apple M4). Never trusted blindly: scored like
                every other candidate
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
        "check" => cmd::check::cmd_check(&a),
        "fix" => cmd::fix::cmd_fix(&a),
        "sheet" => cmd::render::cmd_sheet(&a),
        "compare" => cmd::render::cmd_compare(&a),
        "blind" => cmd::blind::cmd_blind(&a),
        "blind-score" => cmd::blind::cmd_blind_score(&a),
        "probe" => cmd::render::cmd_probe(&a),
        "show" => cmd::render::cmd_show(&a),
        "before-after" => cmd::render::cmd_before_after(&a),
        "closeness" => cmd::check::cmd_closeness(&a),
        "dump" => cmd::data::cmd_dump(&a),
        "fixture" => cmd::data::cmd_fixture(&a),
        other => fail(format!("unknown command {other} (try --help)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_parse_rejects_missing_flag_values_and_keeps_repeated_candidates() {
        assert!(Args::parse(vec!["fix".into(), "in.glb".into(), "--out".into()]).is_err());
        let a = Args::parse(vec![
            "fix".into(),
            "in.glb".into(),
            "--candidate".into(),
            "a.glb".into(),
            "--candidate".into(),
            "b.glb".into(),
            "--out".into(),
            "out.glb".into(),
        ])
        .unwrap();
        assert_eq!(a.many("--candidate"), vec!["a.glb", "b.glb"]);
        assert_eq!(a.get("--out"), Some("out.glb"));
    }

    #[test]
    fn numeric_and_kind_parsers_reject_invalid_values() {
        let bad_top = Args::parse(vec!["show".into(), "--top".into(), "0".into()]).unwrap();
        assert!(args::parse_pos_usize(&bad_top, "--top", 6).is_err());

        let bad_rows = Args::parse(vec!["compare".into(), "--rows".into(), "99".into()]).unwrap();
        assert!(args::parse_range_usize(&bad_rows, "--rows", 5, 1, 12).is_err());

        let bad_kinds = Args::parse(vec!["show".into(), "--kind".into(), " , ".into()]).unwrap();
        assert!(args::parse_kinds(&bad_kinds).is_err());

        let bad_voxels = Args::parse(vec!["check".into(), "--voxels".into(), "abc".into()]).unwrap();
        assert!(args::ctx_opts(&bad_voxels, None).is_err());
    }

    #[test]
    fn flags_of_covers_all_commands() {
        for c in
            ["check", "fix", "sheet", "compare", "blind", "blind-score", "probe", "show", "before-after", "closeness", "dump", "fixture"]
        {
            assert!(flags_of(c).is_some(), "missing flags_of entry for {c}");
        }
        assert!(flags_of("unknown").is_none());
    }
}
