use std::path::{Path, PathBuf};
use std::process::ExitCode;
use weightforge_core as wf;

use crate::args::{Args, ctx_opts, load_poses, motion_spec, parse_kinds, parse_pos_usize, parse_u64};
use crate::common::{fail, find_gk, format_sub_err, winner_of, write_json, write_json_pretty, write_png};

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

    // The planted bad case, made from rig A on the judged pose group: one
    // mirrored role pair swapped, shown at the frame where it tears most.
    let Some((planted_w, planted_pose, planted_verts, planted_what)) = wf::blind::planted_bad(&ca.model, ctxa, use_clips) else {
        return fail("blind: rig A has no mirrored limb pair to plant a bad case from");
    };
    let planted_ev = wf::metrics::evaluate_with(&ca.model, ctxa, &planted_w, false);
    let planted_score = wf::report::build("planted", &ca.model, ctxa, &planted_ev).score;
    let planted_torn = wf::owner::torn(&ca.model, ctxa, &planted_w, use_clips);
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
        tears_cm2: std::collections::BTreeMap<String, f64>,
        tears_winner: String,
        score_winner: String,
        expected: String,
        limg: wf::render::Image,
        rimg: wf::render::Image,
    }

    let sa = wf::blind::BlindSide { model: &ca.model, ctx: ctxa, weights: &ca.model.weights, paint: &paint_a };
    let sb = wf::blind::BlindSide { model: &cb.model, ctx: ctxb, weights: &cb.model.weights, paint: &paint_b };
    let sp = wf::blind::BlindSide { model: &ca.model, ctx: ctxa, weights: &planted_w, paint: &paint_a };

    // A, B and the planted copy of A at one frame, through one camera
    // framed on `marked_idxs` of the side `view_src` (0 A, 1 B, 2 planted).
    let render_triple = |pose: usize, view_src: usize, marked_idxs: &[u32]| -> (wf::render::Image, wf::render::Image, wf::render::Image) {
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
        (img_a, img_b, img_p)
    };

    let mut claim_items: Vec<Pending> = Vec::new();
    let score_w = winner_of("A", "B", ca.rep.score, cb.rep.score, 1.0, false);
    for (idx, c) in claims.iter().enumerate() {
        let (ta, tb) = (torn_cm2(c.src, &c.verts, 0), torn_cm2(c.src, &c.verts, 1));
        let tw = winner_of("A", "B", ta, tb, 0.5, true);
        let (limg, rimg, _) = render_triple(c.pose, c.src, &c.verts);
        let safe: String = format!("{}_{}", c.kind, c.part).chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect();
        claim_items.push(Pending {
            stem: format!("{:03}_{safe}", idx + 1),
            category: "claim",
            focus: format!("{}: {}", c.kind.replace('_', " "), c.part),
            frame: c.frame.clone(),
            tears_cm2: [("A".to_string(), (ta * 10.0).round() / 10.0), ("B".to_string(), (tb * 10.0).round() / 10.0)].into_iter().collect(),
            tears_winner: tw.clone(),
            score_winner: score_w.clone(),
            expected: tw,
            limg,
            rimg,
        });
    }
    if camera == wf::blind::Camera::Talk {
        let (limg, rimg, _) = render_triple(0, 0, &[]);
        claim_items.push(Pending {
            stem: "999_look_face".into(),
            category: "look",
            focus: "the face and head at the talk camera".into(),
            frame: ctxa.poses.first().map(|p| p.name.clone()).unwrap_or_else(|| "rest".into()),
            tears_cm2: std::collections::BTreeMap::new(),
            tears_winner: "tie".into(),
            score_winner: score_w.clone(),
            expected: "tie".into(),
            limg,
            rimg,
        });
    }

    // At the talk camera the planted case is shown where the swap shows
    // most inside the talk frame, on an upright frame; at the follow
    // camera, at the frame where the swap tears most. Never a frame
    // picked for pixel difference or one made up for the planted case.
    let talk_planted_pose = if camera == wf::blind::Camera::Talk {
        let rest = &ca.model.rest;
        let floor = rest.iter().map(|q| q.y).fold(f64::MAX, f64::min);
        let h = wf::owner::height(&ca.model).max(1e-9);
        let (lo, _) = wf::blind::talk_frame_span();
        let in_frame: Vec<usize> = (0..rest.len()).filter(|&i| (rest[i].y - floor) / h >= lo).collect();
        let mut best = (planted_pose, -1.0f64);
        for (i, mats) in ctxa.mats.iter().enumerate() {
            let a_ = wf::skin::lbs(&ca.model, mats, &ca.model.weights);
            if wf::blind::head_share(rest, &a_) < 0.6 {
                continue;
            }
            let p_ = wf::skin::lbs(&ca.model, mats, &planted_w);
            let d: f64 = in_frame.iter().map(|&v| (p_[v] - a_[v]).len()).sum();
            if d > best.1 {
                best = (i, d);
            }
        }
        best.0
    } else {
        planted_pose
    };
    let planted_frame = ctxa.poses.get(talk_planted_pose).map(|p| p.name.clone()).unwrap_or_else(|| "rest".into());

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

    // `gk blind make --planted` pairs stage/a's first frame with the
    // planted image, so A's whole body at the planted frame goes first.
    let stage_planted = stage.join("planted.png");
    {
        let ta_all: f64 = (0..ca.model.nverts()).filter(|&v| torna[v]).map(|v| areaa[v] * ka * ka * 1e4).sum();
        let tb_all: f64 = (0..cb.model.nverts()).filter(|&v| tornb[v]).map(|v| areab[v] * kb * kb * 1e4).sum();
        let tw_all = winner_of("A", "B", ta_all, tb_all, 0.5, true);
        let (limg, rimg, pimg) = render_triple(talk_planted_pose, 2, &planted_verts);
        if let Err(e) = write_png(&stage_planted, &pimg) {
            cleanup(&stage);
            return fail(e);
        }
        claim_items.insert(
            0,
            Pending {
                stem: "000_whole_body".into(),
                category: "claim",
                focus: "the whole body at the game camera".into(),
                frame: planted_frame.clone(),
                tears_cm2: [("A".to_string(), (ta_all * 10.0).round() / 10.0), ("B".to_string(), (tb_all * 10.0).round() / 10.0)]
                    .into_iter()
                    .collect(),
                tears_winner: tw_all.clone(),
                score_winner: score_w.clone(),
                expected: tw_all,
                limg,
                rimg,
            },
        );
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

    let pta: f64 = planted_verts
        .iter()
        .filter(|&&v| torna.get(v as usize).copied().unwrap_or(false))
        .map(|&v| areaa.get(v as usize).copied().unwrap_or(0.0) * ka * ka * 1e4)
        .sum();
    let ptp: f64 = planted_verts
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
        kobj.insert("planted".into(), serde_json::json!(planted_what));
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
