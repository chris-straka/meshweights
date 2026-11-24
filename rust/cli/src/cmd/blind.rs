use std::path::{Path, PathBuf};
use std::process::ExitCode;
use weightforge_core as wf;

use crate::args::{Args, ctx_opts, load_poses, motion_spec, parse_kinds, parse_pos_usize, parse_u64};
use crate::common::{fail, find_gk, format_sub_err, image_rmse, winner_of, write_json, write_json_pretty, write_png};

/// Same-frame blind pairs per claim at the game camera (delegated to `gk blind make`).
pub fn cmd_blind(a: &Args) -> ExitCode {
    let (Some(pa), Some(pb)) = (a.pos.get(1), a.pos.get(2)) else { return fail("blind needs two .glb files") };
    let Some(dir) = a.get("--out").map(Path::new) else { return fail("blind needs --out <dir>") };
    let top = match parse_pos_usize(a, "--top", 6) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let seed = match parse_u64(a, "--seed", 20261115) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    let kinds = match parse_kinds(a) {
        Ok(k) => k,
        Err(e) => return fail(e),
    };
    let camera = match a.get("--camera").map(wf::blind::Camera::parse) {
        None => wf::blind::Camera::Follow,
        Some(Some(c)) => c,
        Some(None) => return fail("--camera is follow or talk"),
    };
    // The follow camera's lens and distance, from --metrics or $METRICS_FILE.
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
    let spec = motion_spec(a);
    let gait = a.has("--gait");
    let (ca, cb) =
        match (wf::check_full_with_gait(Path::new(pa), &opts, &spec, gait), wf::check_full_with_gait(Path::new(pb), &opts, &spec, gait)) {
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
                if let Some(slot) = t.get_mut(v as usize) {
                    *slot = true;
                }
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
            .filter(|&&v| torn.get(v as usize).copied().unwrap_or(false))
            .map(|&v| area.get(v as usize).copied().unwrap_or(0.0) * k * k * 1e4)
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

    let render_triple_with = |sctx_a: &wf::metrics::Ctx,
                              sctx_b: &wf::metrics::Ctx,
                              planted_w: &wf::scene::Weights,
                              pose: usize,
                              view_src: usize,
                              marked_idxs: &[u32]|
     -> (wf::render::Image, wf::render::Image, wf::render::Image, f64) {
        let sa_side = wf::blind::BlindSide { model: &ca.model, ctx: sctx_a, weights: &ca.model.weights, paint: &paint_a };
        let sb_side = wf::blind::BlindSide { model: &cb.model, ctx: sctx_b, weights: &cb.model.weights, paint: &paint_b };
        let sp_side = wf::blind::BlindSide { model: &ca.model, ctx: sctx_a, weights: planted_w, paint: &paint_a };
        let vs = if view_src == 1 {
            &sb_side
        } else if view_src == 2 {
            &sp_side
        } else {
            &sa_side
        };
        let src_model = if view_src == 1 { &cb.model } else { &ca.model };
        let mut marked = vec![false; vs.model.nverts()];
        for &v in wf::blind::map_vertices(src_model, vs.model, marked_idxs).iter() {
            if let Some(slot) = marked.get_mut(v as usize) {
                *slot = true;
            }
        }
        let posed = |s: &wf::blind::BlindSide<'_>| {
            if s.ctx.mats.is_empty() {
                s.model.rest.clone()
            } else {
                wf::skin::lbs(s.model, &s.ctx.mats[pose.min(s.ctx.mats.len() - 1)], s.weights)
            }
        };
        let (pa_p, pb_p) = (posed(&sa_side), posed(&sb_side));
        let (view, fit) = match camera {
            wf::blind::Camera::Follow => {
                let view = wf::blind::claim_view(vs, pose, &marked);
                let fit = wf::blind::game_fit(&view, &[&pa_p, &pb_p], wf::owner::height(sa_side.model), 520, 640, share);
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
        let img_a = wf::blind::cell_image(&sa_side, pose, &view, fit, 520, 640, "");
        let img_b = wf::blind::cell_image(&sb_side, pose, &view, fit, 520, 640, "");
        let img_p = wf::blind::cell_image(&sp_side, pose, &view, fit, 520, 640, "");
        let rmse = image_rmse(&img_a, &img_p);
        (img_a, img_b, img_p, rmse)
    };
    let render_triple = |planted_w: &wf::scene::Weights, pose: usize, view_src: usize, marked_idxs: &[u32]| {
        render_triple_with(ctxa, ctxb, planted_w, pose, view_src, marked_idxs)
    };
    let _ = (&sa, &sb);

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
            frame: ctxa.poses.first().map(|p| p.name.clone()).unwrap_or_else(|| "rest".into()),
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

    let rest = &ca.model.rest;
    let floor = rest.iter().map(|q| q.y).fold(f64::MAX, f64::min);
    let h = wf::owner::height(&ca.model).max(1e-9);
    let (talk_lo, _) = wf::blind::talk_frame_span();
    let talk_verts: Vec<usize> = (0..rest.len()).filter(|&i| (rest[i].y - floor) / h >= talk_lo).collect();

    struct PlantedPick<'a> {
        cand: &'a wf::blind::PlantedCandidate,
        pose: usize,
        limg: wf::render::Image,
        rimg: wf::render::Image,
        pimg: wf::render::Image,
        rmse: f64,
        asym: f64,
        score: f64,
        frame: String,
        custom_ctx: Option<(wf::metrics::Ctx, wf::metrics::Ctx)>,
    }

    let asym_weight = |rmse: f64, asym: f64| -> f64 { rmse * (0.25 + asym.min(0.60)) };
    let mut best_pick: Option<PlantedPick<'_>> = None;
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
                    .filter_map(|&v| rest.get(v).zip(ca.model.weights.get(v)).zip(cand.weights.get(v)).map(|((r, wa), wp)| (r, wa, wp)))
                    .map(|(&r, wa, wp)| {
                        let pa = wf::skin::lbs_vertex(mats, wa, r);
                        let pp = wf::skin::lbs_vertex(mats, wp, r);
                        (pp - pa).len()
                    })
                    .sum();
                let a_val = wf::blind::swap_asymmetry(&ca.model, mats, &cand.swap);
                (i, d * (0.20 + a_val))
            })
            .collect();
        ranked_poses.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut try_poses: Vec<usize> = ranked_poses.into_iter().take(6).map(|(p, _)| p).collect();
        if (!any_upright || upright.get(cand.pose).copied().unwrap_or(false)) && !try_poses.contains(&cand.pose) {
            try_poses.push(cand.pose);
        }
        for pose in try_poses {
            let (limg, rimg, pimg, rmse) = render_triple(&cand.weights, pose, 2, &cand.vertices);
            let asym = wf::blind::swap_asymmetry(&ca.model, &ctxa.mats[pose], &cand.swap);
            let score = asym_weight(rmse, asym);
            let replace = best_pick.as_ref().is_none_or(|b| {
                let b_vis = b.rmse >= 8.0;
                let n_vis = rmse >= 8.0;
                match (b_vis, n_vis) {
                    (false, true) => true,
                    (true, false) => false,
                    _ => score > b.score,
                }
            });
            if replace {
                let frame = ctxa.poses.get(pose).map(|p| p.name.clone()).unwrap_or_else(|| "rest".into());
                best_pick = Some(PlantedPick { cand, pose, limg, rimg, pimg, rmse, asym, score, frame, custom_ctx: None });
            }
        }
        if best_pick.as_ref().is_some_and(|b| b.rmse >= 12.0 && b.asym >= 0.12) {
            break;
        }
    }

    // If all input frames were mirror-symmetric (e.g. symmetric lunges only),
    // synthesize an upright opposite-phase stride frame so swaps read clearly.
    if best_pick.as_ref().is_none_or(|b| b.asym < 0.08 || b.rmse < 8.0) {
        for cand in &planted_cands {
            let Some(asym_pose) = wf::blind::asymmetric_planted_pose(&ca.model, &cand.swap) else {
                continue;
            };
            let pose_b = if ca.model.skel.names == cb.model.skel.names {
                asym_pose.clone()
            } else {
                let moves = asym_pose
                    .moves
                    .iter()
                    .filter_map(|&(j, ax, ang)| {
                        ca.model.skel.names.get(j).and_then(|nm| cb.model.skel.joint_by_name(nm)).map(|bj| (bj, ax, ang))
                    })
                    .collect();
                wf::poses::Pose { name: asym_pose.name.clone(), moves, locals: Vec::new() }
            };
            let mut poses_a = ctxa.poses.clone();
            let pose_idx = poses_a.len();
            poses_a.push(asym_pose.clone());
            let c_a = ctxa.with_poses(&ca.model, poses_a);
            let mut poses_b = ctxb.poses.clone();
            poses_b.push(pose_b);
            let c_b = ctxb.with_poses(&cb.model, poses_b);
            if wf::blind::is_upright_pose(&ca.model, &c_a.mats[pose_idx]) || !any_upright {
                let (limg, rimg, pimg, rmse) = render_triple_with(&c_a, &c_b, &cand.weights, pose_idx, 2, &cand.vertices);
                let asym = wf::blind::swap_asymmetry(&ca.model, &c_a.mats[pose_idx], &cand.swap);
                let score = asym_weight(rmse, asym);
                let replace = best_pick.as_ref().is_none_or(|b| (rmse >= 6.0 && b.rmse < 8.0) || score > b.score);
                if replace {
                    best_pick = Some(PlantedPick {
                        cand,
                        pose: pose_idx,
                        limg,
                        rimg,
                        pimg,
                        rmse,
                        asym,
                        score,
                        frame: asym_pose.name.clone(),
                        custom_ctx: Some((c_a, c_b)),
                    });
                    if rmse >= 12.0 {
                        break;
                    }
                }
            }
        }
    }

    // Only reuse an existing claim frame for `planted` when it is upright, visible,
    // and genuinely asymmetric relative to the best dedicated asymmetric frame.
    let best_ded_score = best_pick.as_ref().map_or(0.0, |b| b.score);
    let best_ded_asym = best_pick.as_ref().map_or(0.0, |b| b.asym);
    let best_claim_idx = claim_items
        .iter()
        .enumerate()
        .filter_map(|(i, c)| {
            if (any_upright && !upright.get(c.pose).copied().unwrap_or(false)) || c.planted_rmse <= 8.0 {
                return None;
            }
            let casym = ctxa.mats.get(c.pose).map(|m| wf::blind::swap_asymmetry(&ca.model, m, &primary_planted.swap)).unwrap_or(0.0);
            if casym < 0.08 && best_ded_asym >= 0.08 {
                return None;
            }
            let cscore = asym_weight(c.planted_rmse, casym);
            (cscore >= 0.85 * best_ded_score).then_some((i, cscore))
        })
        .max_by(|(_, x), (_, y)| x.total_cmp(y))
        .map(|(i, _)| i);

    let stage_planted = stage.join("planted.png");
    let (chosen_planted, planted_frame, custom_planted_ctx): (&wf::blind::PlantedCandidate, String, Option<wf::metrics::Ctx>) =
        if let Some(idx) = best_claim_idx {
            let tail = claim_items[idx].stem.get(4..).unwrap_or(&claim_items[idx].stem).to_string();
            claim_items[idx].stem = format!("000_{tail}");
            if let Err(e) = write_png(&stage_planted, &claim_items[idx].pimg) {
                cleanup(&stage);
                return fail(e);
            }
            (primary_planted, claim_items[idx].frame.clone(), None)
        } else {
            let Some(pick) = best_pick else {
                cleanup(&stage);
                return fail("blind: no pose available to render planted bad case");
            };
            let ta_all: f64 = (0..ca.model.nverts()).filter(|&v| torna[v]).map(|v| areaa[v] * ka * ka * 1e4).sum();
            let tb_all: f64 = (0..cb.model.nverts()).filter(|&v| tornb[v]).map(|v| areab[v] * kb * kb * 1e4).sum();
            let tw_all = winner_of("A", "B", ta_all, tb_all, 0.5, true);
            if let Err(e) = write_png(&stage_planted, &pick.pimg) {
                cleanup(&stage);
                return fail(e);
            }
            let c_a_opt = pick.custom_ctx.map(|(a_c, _)| a_c);
            let planted_rmse = pick.rmse;
            let frame_name = pick.frame.clone();
            claim_items.insert(
                0,
                Pending {
                    stem: "000_whole_body".into(),
                    category: "claim",
                    focus: "the whole body at the game camera".into(),
                    frame: frame_name.clone(),
                    pose: pick.pose,
                    view_src: 2,
                    marked: pick.cand.vertices.clone(),
                    tears_cm2: [("A".to_string(), (ta_all * 10.0).round() / 10.0), ("B".to_string(), (tb_all * 10.0).round() / 10.0)]
                        .into_iter()
                        .collect(),
                    tears_winner: tw_all.clone(),
                    score_winner: score_w.clone(),
                    expected: tw_all,
                    limg: pick.limg,
                    rimg: pick.rimg,
                    pimg: pick.pimg,
                    planted_rmse,
                },
            );
            (pick.cand, frame_name, c_a_opt)
        };
    for c in &claim_items {
        let _ = (c.pose, c.view_src, c.marked.len(), c.planted_rmse);
    }

    for c in &claim_items {
        if let Err(e) = write_png(&stage_a.join(format!("{}.png", c.stem)), &c.limg)
            .and_then(|_| write_png(&stage_b.join(format!("{}.png", c.stem)), &c.rimg))
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

    let planted_eval_ctx = custom_planted_ctx.as_ref().unwrap_or(ctxa);
    let planted_ev = wf::metrics::evaluate_with(&ca.model, planted_eval_ctx, &chosen_planted.weights, false);
    let planted_score = wf::report::build("planted", &ca.model, planted_eval_ctx, &planted_ev).score;
    let planted_torn = wf::owner::torn(&ca.model, planted_eval_ctx, &chosen_planted.weights, use_clips);
    let pta: f64 = chosen_planted
        .vertices
        .iter()
        .filter(|&&v| torna.get(v as usize).copied().unwrap_or(false))
        .map(|&v| areaa.get(v as usize).copied().unwrap_or(0.0) * ka * ka * 1e4)
        .sum();
    let ptp: f64 = chosen_planted
        .vertices
        .iter()
        .filter(|&&v| planted_torn.get(v as usize).copied().unwrap_or(false))
        .map(|&v| areaa.get(v as usize).copied().unwrap_or(0.0) * ka * ka * 1e4)
        .sum();
    let planted_tears_winner = winner_of("A", "PLANTED_A", pta, ptp, 0.5, true);
    let planted_score_winner = winner_of("A", "PLANTED_A", ca.rep.score, planted_score, 1.0, false);

    let Some(c0) = claim_items.first() else {
        cleanup(&stage);
        return fail("blind: no pairs generated");
    };
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
    if let Err(e) = write_json_pretty(&key_path, serde_json::to_string_pretty(&kdoc)) {
        cleanup(&stage);
        return fail(e);
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
pub fn cmd_blind_score(a: &Args) -> ExitCode {
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
        if let Err(e) = write_json(&tmp_ans, serde_json::to_vec(&serde_json::Value::Object(norm_by_id))) {
            return fail(e);
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
    if let Err(e) = write_json_pretty(&out, serde_json::to_string_pretty(&v)) {
        return fail(e);
    }
    println!("{}", out.display());
    if v.pass { ExitCode::SUCCESS } else { ExitCode::from(1) }
}
