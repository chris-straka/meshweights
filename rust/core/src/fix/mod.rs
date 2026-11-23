//! Weight repair. Builds candidate weight sets, scores each on the same pose
//! set, keeps the best candidate per broken region, blends at region seams,
//! and never returns anything that scores worse than the input in any
//! region (the input is always candidate 0).

use crate::metrics::{Ctx, Eval, evaluate, evaluate_partial, evaluate_with};
use crate::report::{Report, build, region_energy, region_energy_of};
use crate::scene::{Model, Weights};
use serde::Serialize;
use std::collections::BTreeMap;

mod candidates;
mod helpers;
mod picking;

pub use candidates::{cleanup, crotch_to_pelvis, external, geodesic, joint_bands, map_joints, mirror, optimize, own_bones, transfer};
pub use helpers::{dilate, distances, grow, passes, smooth, widths};

use helpers::{drop_small_clusters, is_piece};
use picking::{EDIT_MU, blend, fail_limits, flag_counts, l1, summary};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Method {
    Smooth,
    Geodesic,
    Transfer,
    Optimize,
}

impl Method {
    /// `auto`, one method, or a comma-separated list (`smooth,geodesic`).
    pub fn parse(s: &str) -> Option<Vec<Method>> {
        if s.contains(',') {
            let mut out = Vec::new();
            for part in s.split(',') {
                for m in Method::parse(part.trim())? {
                    if !out.contains(&m) {
                        out.push(m);
                    }
                }
            }
            return Some(out);
        }
        Some(match s {
            "auto" => vec![Method::Smooth, Method::Geodesic, Method::Transfer, Method::Optimize],
            "smooth" | "cleanup" => vec![Method::Smooth],
            "geodesic" => vec![Method::Geodesic],
            "transfer" => vec![Method::Transfer],
            "optimize" => vec![Method::Optimize],
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Method::Smooth => "smooth",
            Method::Geodesic => "geodesic",
            Method::Transfer => "transfer",
            Method::Optimize => "optimize",
        }
    }
}

pub struct FixOpts<'a> {
    pub methods: Vec<Method>,
    /// Known-good rigged base to transfer from (e.g. wrapforge's fit).
    pub source: Option<&'a Model>,
    /// Extra candidates (e.g. SkinTokens output), scored like the rest.
    pub external: Vec<(String, &'a Model)>,
    /// Let regions without findings change too (default: fix only what
    /// the check flags).
    pub all_regions: bool,
    /// Judge the result by the headline (`owner::guard`): regions where it
    /// tears more skin than the input, on the poses or on `motion`'s clips,
    /// keep the input's weights.
    pub headline_guard: bool,
    pub motion: Option<&'a Ctx>,
}

impl Default for FixOpts<'_> {
    fn default() -> Self {
        FixOpts {
            methods: Method::parse("auto").unwrap(),
            source: None,
            external: Vec::new(),
            all_regions: false,
            headline_guard: true,
            motion: None,
        }
    }
}

pub struct Candidate {
    pub name: String,
    pub w: Weights,
    pub region_e: Vec<f64>,
}

#[derive(Serialize)]
pub struct RegionChoice {
    pub name: String,
    pub label: String,
    pub before: f64,
    pub after: f64,
    pub chosen: String,
    pub candidates: BTreeMap<String, f64>,
}

#[derive(Serialize)]
pub struct Summary {
    pub score: f64,
    pub pass: bool,
    pub fails: usize,
}

#[derive(Serialize)]
pub struct FixReport {
    pub tool: String,
    pub version: String,
    pub input: String,
    pub methods: Vec<String>,
    pub before: Summary,
    pub after: Summary,
    pub improved: bool,
    /// The region pick searched every plan (false: it hit its node budget
    /// and kept the best plan found; only on rigs with many failing regions).
    pub pick_exact: bool,
    /// False when the run without the external candidates scored better
    /// and was kept (`fix`).
    pub external_kept: bool,
    pub verts_changed: usize,
    pub mean_l1_change: f64,
    pub max_influences: usize,
    pub candidates: BTreeMap<String, f64>,
    pub regions: Vec<RegionChoice>,
    pub findings_before: Vec<crate::report::Finding>,
    pub findings_after: Vec<crate::report::Finding>,
    /// Tears (% of the surface in failing headline findings) per pose
    /// group, before and after; empty with `--no-headline-guard`. The fix
    /// never returns more than the input on any group.
    pub tears_before: BTreeMap<String, f64>,
    pub tears_after: BTreeMap<String, f64>,
}

pub struct FixResult {
    pub weights: Weights,
    pub eval: Eval,
    pub report_before: Report,
    pub report_after: Report,
    pub fix: FixReport,
    /// Skin inside failing headline findings after the fix (cm^2 on a
    /// 1.75 m body, poses and clips together), when the guard judged.
    pub torn_cm2: Option<f64>,
}

/// Repairs the weights (see the module docs). With external candidates
/// it also runs without them and keeps the better result (passes the gate
/// first, then score): an extra candidate never makes the fix worse than
/// not having it, even where the region pick runs out of search budget.
pub fn fix(file: &str, model: &Model, ctx: &Ctx, o: &FixOpts) -> FixResult {
    let with = fix_once(file, model, ctx, o);
    if o.external.is_empty() {
        return with;
    }
    let plain = FixOpts {
        methods: o.methods.clone(),
        source: o.source,
        external: Vec::new(),
        all_regions: o.all_regions,
        headline_guard: o.headline_guard,
        motion: o.motion,
    };
    let mut without = fix_once(file, model, ctx, &plain);
    let key = |r: &FixResult| (r.report_after.pass, r.report_after.score);
    let better_without = if with.report_after.pass != without.report_after.pass {
        without.report_after.pass
    } else {
        match (with.torn_cm2, without.torn_cm2) {
            // Judged by what is seen when the guard is on; when tears tie,
            // the score still decides (`an_external_candidate_never_hurts`).
            (Some(a), Some(b)) if (a - b).abs() > 1e-6 => b < a,
            _ => key(&without) > key(&with),
        }
    };
    if better_without {
        without.fix.methods = with.fix.methods.clone();
        without.fix.external_kept = false;
        return without;
    }
    with
}

fn fix_once(file: &str, model: &Model, ctx: &Ctx, o: &FixOpts) -> FixResult {
    let ev0 = evaluate(model, ctx, &model.weights);
    let rep0 = build(file, model, ctx, &ev0);
    let score = |w: &Weights| -> (Eval, Vec<f64>) {
        let ev = evaluate_with(model, ctx, w, false);
        let re = region_energy(ctx, &ev);
        (ev, re)
    };
    // Skin the headline sees torn in the input, on the poses or the clips:
    // with the headline judging, it opens its region for change like a
    // failing finding does (character line round 3: a cut body with no
    // failing finding on the old score but a flap torn in the run got no
    // change at all).
    let torn0: Vec<bool> = if o.headline_guard {
        let mut t = crate::owner::torn(model, ctx, &model.weights, false);
        if let Some(m) = o.motion {
            for (a, b) in t.iter_mut().zip(crate::owner::torn(model, m, &model.weights, true)) {
                *a |= b;
            }
        }
        t
    } else {
        vec![false; model.nverts()]
    };
    // Regions eligible for change: those with failing findings or torn skin.
    let nr = ctx.regions.len();
    let mut failing: Vec<bool> =
        ctx.regions.iter().map(|r| o.all_regions || rep0.findings.iter().any(|f| f.severity == "fail" && f.region == r.name)).collect();
    for v in (0..model.nverts()).filter(|&v| torn0[v]) {
        failing[ctx.vregion[v] as usize] = true;
    }
    // Only the reported bad vertices (plus a margin) change; a failing
    // piece changes as a whole. The rest of a region keeps its weights.
    let mask = crate::report::finding_mask(ctx, &ev0, &rep0);
    // In a failing region the strained skin around the tear opens too
    // (stretch past 0.7 of the limit): the vertices over the line alone
    // are too small an area to rebuild a joint's band in.
    let strained = |v: usize| failing[ctx.vregion[v] as usize] && ev0.max_stretch[v] > 0.7 * ctx.th.stretch;
    let flagged: Vec<bool> = (0..model.nverts())
        .map(|v| {
            o.all_regions
                || torn0[v]
                || mask[v] & !crate::metrics::F_INTERSECT != 0
                || strained(v)
                || (is_piece(model, v) && failing[ctx.vregion[v] as usize])
                // Speckle that does not show yet beside speckle that does
                // shows once its neighbours are cleaned.
                || (failing[ctx.vregion[v] as usize] && ev0.speckle.get(v).is_some_and(|&x| x > ctx.th.noise))
        })
        .collect();
    // Isolated flags (a stray vertex or two) do not open an area for change.
    let flagged = drop_small_clusters(model, &flagged, ctx.th.region_min_bad);
    let active = grow(model, &flagged, widths::MARGIN * ctx.feature, None);
    let seam = widths::SEAM * ctx.feature;
    if std::env::var_os("WF_DEBUG").is_some() {
        eprintln!(
            "fix: {} verts, {} active, feature {:.4}, seam {seam:.4}",
            model.nverts(),
            active.iter().filter(|&&a| a).count(),
            ctx.feature
        );
    }
    // Incremental trial scoring: only regions touching a changed vertex (or
    // its one-ring), and pieces hanging on changed body triangles, are
    // re-evaluated; every other vertex keeps its cached energy.
    let score_trial = |wt: &Weights, base: &Weights, base_v: &[f64]| -> (Vec<f64>, Vec<u8>, Vec<bool>) {
        let mut dirty_r = vec![false; ctx.regions.len()];
        let mut changed = vec![false; model.nverts()];
        let mut bones_changed = false;
        for v in 0..model.nverts() {
            if wt[v] != base[v] {
                changed[v] = true;
                dirty_r[ctx.vregion[v] as usize] = true;
                for &u in &model.adj[v] {
                    dirty_r[ctx.vregion[u as usize] as usize] = true;
                }
                let bones = |w: &crate::scene::VW| w.iter().filter(|e| e.1 > 0.0).map(|e| e.0).collect::<std::collections::BTreeSet<u32>>();
                bones_changed |= bones(&wt[v]) != bones(&base[v]);
            }
        }
        // Not only neighbours: a piece hangs on body triangles it shares no
        // edge with (its follow energy reads their weights), and which
        // pieces use non-body bones depends on every body vertex (review
        // 2026-11-13).
        for &(pv, t, _, ok) in &ctx.piece_matches {
            if ok && t.iter().any(|&i| changed[i as usize]) {
                dirty_r[ctx.vregion[pv as usize] as usize] = true;
            }
        }
        if bones_changed {
            for (r, reg) in ctx.regions.iter().enumerate() {
                if reg.piece {
                    dirty_r[r] = true;
                }
            }
        }
        let mask: Vec<bool> = (0..model.nverts()).map(|v| dirty_r[ctx.vregion[v] as usize]).collect();
        let part = evaluate_partial(model, ctx, wt, &mask);
        let e: Vec<f64> = (0..model.nverts()).map(|v| if mask[v] { part.energy[v] } else { base_v[v] }).collect();
        (region_energy_of(ctx, &e), part.flags, dirty_r)
    };
    let mut cands: Vec<Candidate> = Vec::new();
    // Candidates are scored as they will be applied: only on the active
    // area (seam-blended), the input everywhere else.
    let push = |name: String, mut w: Weights, cands: &mut Vec<Candidate>| {
        model.lock_rigid(&mut w);
        let local = if cands.is_empty() {
            w.clone()
        } else {
            let pair = [
                Candidate { name: String::new(), w: model.weights.clone(), region_e: Vec::new() },
                Candidate { name: String::new(), w: w.clone(), region_e: Vec::new() },
            ];
            blend(model, ctx, &pair, &vec![1; nr], &active, seam)
        };
        let (_, region_e) = score(&local);
        cands.push(Candidate { name, w, region_e });
    };
    push("original".into(), model.weights.clone(), &mut cands);
    let match_dist = 0.03;
    for &m in &o.methods {
        match m {
            Method::Smooth => {
                push("despeckle".into(), cleanup(model, ctx, &ev0, false), &mut cands);
                push("smooth".into(), cleanup(model, ctx, &ev0, true), &mut cands);
                if o.headline_guard {
                    push("own-bones".into(), own_bones(model, ctx), &mut cands);
                }
            }
            Method::Geodesic => {
                push("geodesic".into(), geodesic(model, ctx), &mut cands);
                push("geodesic-band".into(), joint_bands(model, ctx), &mut cands);
            }
            Method::Transfer => push(
                if o.source.is_some() { "transfer".into() } else { "transfer(pieces)".into() },
                transfer(model, ctx, o.source, match_dist),
                &mut cands,
            ),
            Method::Optimize => {}
        }
    }
    // Twist/helper bones: generic candidates leave them unweighted (they
    // carry no weight in the input); the helper band weights them, alone
    // and refined by optimize (helpers.rs).
    if !model.skel.helpers.is_empty() {
        // How wide an armpit or groin band should be depends on the body
        // and the rig (wider trades stretch for collapse), so three widths
        // are scored and the pick keeps whichever repairs best.
        for (tag, width) in [("", 1.0), ("-narrow", 0.7), ("-wide", 1.4)] {
            let bw = crate::helpers::band(model, ctx, &model.weights, width);
            if o.methods.contains(&Method::Optimize) {
                let (ev_b, _) = score(&bw);
                push(format!("helper-band{tag}+optimize"), optimize(model, ctx, &bw, &ev_b), &mut cands);
            }
            push(format!("helper-band{tag}"), bw, &mut cands);
        }
    }
    // Limbs cut apart by meshkit's unfuse (it records its cuts in the
    // file): the crotch's falloff on top of each half following its own
    // leg, and the same for an arm cut free (unfuse.rs). The caller has
    // already given each half to its own leg (`unfuse::cut_start`: the
    // weights a cut leaves are interpolated across it and mean nothing).
    // A body that was not cut gets no such candidate.
    let apart = crate::unfuse::limbs_apart(model, &model.weights);
    if apart != model.weights {
        if o.methods.contains(&Method::Optimize) {
            let (ev_b, _) = score(&apart);
            push("limbs-apart+optimize".into(), optimize(model, ctx, &apart, &ev_b), &mut cands);
        }
        push("limbs-apart".into(), apart, &mut cands);
    }
    for (name, other) in &o.external {
        push(format!("external:{name}"), external(model, ctx, other, match_dist), &mut cands);
    }

    // -------------------------------------------------------------- picking
    // Moves: one candidate on a set of regions, measured alone against the
    // input (pick.rs). The search over plans is exact, so the result never
    // depends on visiting order and a better candidate never scores worse.
    let orig_ev = evaluate_with(model, ctx, &model.weights, false);
    let orig_v = orig_ev.energy.clone();
    let orig_e = region_energy_of(ctx, &orig_v);
    let mut size = vec![0.0f64; nr];
    for &r in &ctx.vregion {
        size[r as usize] += 1.0;
    }
    let had_fail: Vec<bool> =
        ctx.regions.iter().map(|r| rep0.findings.iter().any(|f| f.severity == "fail" && f.region == r.name)).collect();
    let base_counts = flag_counts(ctx, &orig_ev.flags, None);
    let (fail_at, limit) = fail_limits(model, ctx, &had_fail);
    // Region adjacency (regions sharing a mesh edge).
    let mut adjr = vec![std::collections::BTreeSet::new(); nr];
    for e in &model.edges {
        let (a, b) = (ctx.vregion[e[0] as usize] as usize, ctx.vregion[e[1] as usize] as usize);
        if a != b {
            adjr[a].insert(b);
            adjr[b].insert(a);
        }
    }
    // Region sets a move may cover, per failing region: alone, with its
    // failing neighbors, with all neighbors (a fault across a joint spans
    // two regions), its group of failing regions (failing regions within
    // two steps of each other: both thighs across the hips), and that
    // group with all their neighbors. Only active vertices ever change.
    let mut sets: std::collections::BTreeSet<Vec<usize>> = std::collections::BTreeSet::new();
    let near_all = |r: usize| -> Vec<usize> { std::iter::once(r).chain(adjr[r].iter().copied()).collect() };
    for r in (0..nr).filter(|&r| failing[r]) {
        let near_fail: Vec<usize> = std::iter::once(r).chain(adjr[r].iter().copied().filter(|&k| failing[k])).collect();
        let mut group = vec![r];
        let mut i = 0;
        while i < group.len() {
            let around = near_all(group[i]);
            let join: Vec<usize> =
                (0..nr).filter(|&k| failing[k] && !group.contains(&k) && near_all(k).iter().any(|x| around.contains(x))).collect();
            group.extend(join);
            i += 1;
        }
        let mut wide: Vec<usize> = group.iter().flat_map(|&g| near_all(g)).collect();
        wide.sort_unstable();
        wide.dedup();
        for mut s in [vec![r], near_fail, near_all(r), group, wide] {
            s.sort_unstable();
            sets.insert(s);
        }
    }
    let debug = std::env::var_os("WF_DEBUG").is_some();
    // The state moves are measured against: the input at first, the
    // current plan in the refinement rounds.
    struct Base {
        choice: Vec<usize>,
        w: Weights,
        v: Vec<f64>,
        e: Vec<f64>,
        counts: Vec<[i64; crate::pick::NFLAG]>,
        change: f64,
    }
    let base_of = |choice: Vec<usize>, w: Weights| -> Base {
        let ev = evaluate_with(model, ctx, &w, false);
        let e = region_energy_of(ctx, &ev.energy);
        let counts = flag_counts(ctx, &ev.flags, None);
        let change = (0..model.nverts()).map(|v| l1(&w[v], &model.weights[v])).sum();
        Base { choice, w, v: ev.energy, e, counts, change }
    };
    let input_base = Base {
        choice: vec![0; nr],
        w: model.weights.clone(),
        v: orig_v.clone(),
        e: orig_e.clone(),
        counts: base_counts.clone(),
        change: 0.0,
    };
    let measure = |cands: &[Candidate], c: usize, base: &Base| -> Vec<crate::pick::Move> {
        sets.iter()
            .filter(|members| members.iter().any(|&k| base.choice[k] != c))
            .map(|members| {
                let mut trial = base.choice.clone();
                for &k in members {
                    trial[k] = c;
                }
                let wt = blend(model, ctx, cands, &trial, &active, seam);
                let (re, flags, dirty) = score_trial(&wt, &base.w, &base.v);
                let delta: Vec<(usize, f64)> = (0..nr).filter(|&k| dirty[k]).map(|k| (k, re[k] - base.e[k])).collect();
                let counts = flag_counts(ctx, &flags, Some(&dirty));
                let mut fl = Vec::new();
                for k in (0..nr).filter(|&k| dirty[k]) {
                    for (f, (&cnt, &bcnt)) in counts[k].iter().zip(&base.counts[k]).enumerate() {
                        let d = cnt - bcnt;
                        if d != 0 {
                            fl.push((k, f, d));
                        }
                    }
                }
                // Weight change vs the input added by this move (0 if it
                // brings the weights back toward the input).
                let change: f64 = (0..model.nverts()).map(|v| l1(&wt[v], &model.weights[v])).sum::<f64>() - base.change;
                crate::pick::Move { cand: c, members: members.clone(), delta, flags: fl, change: change.max(0.0) }
            })
            .collect()
    };
    // Search order: outward from the worst failing region along region
    // adjacency, so moves that conflict are decided next to each other.
    let mut by_e: Vec<usize> = (0..nr).filter(|&r| failing[r]).collect();
    by_e.sort_by(|&a, &b| orig_e[b].total_cmp(&orig_e[a]).then(a.cmp(&b)));
    let mut anchors: Vec<usize> = Vec::new();
    let mut seen = vec![false; nr];
    for &start in &by_e {
        if seen[start] {
            continue;
        }
        let mut queue = std::collections::VecDeque::from([start]);
        seen[start] = true;
        while let Some(r) = queue.pop_front() {
            if failing[r] {
                anchors.push(r);
            }
            for &k in &adjr[r] {
                if !seen[k] {
                    seen[k] = true;
                    queue.push_back(k);
                }
            }
        }
    }
    let problem_at = |base: &Base, moves: Vec<crate::pick::Move>, slack: f64| crate::pick::Problem {
        orig: base.e.clone(),
        size: size.clone(),
        // Rules stay relative to the input: caps on its energies, no
        // region gains a failing finding it did not have.
        cap: orig_e.iter().map(|e| e * 1.1 + slack).collect(),
        counts: base.counts.clone(),
        fail_at: fail_at.clone(),
        limit: limit.clone(),
        anchors: anchors.clone(),
        moves,
        gate_first: true,
        edit_cost: EDIT_MU / size.iter().sum::<f64>().max(1.0),
        min_gain: 0.2,
    };
    let problem = |moves: Vec<crate::pick::Move>, slack: f64| problem_at(&input_base, moves, slack);
    let apply = |cands: &[Candidate], p: &crate::pick::Problem, plan: &crate::pick::Plan, from: &[usize]| -> (Vec<usize>, Weights) {
        let mut choice = from.to_vec();
        for &mi in &plan.moves {
            for &k in &p.moves[mi].members {
                choice[k] = p.moves[mi].cand;
            }
        }
        let w = blend(model, ctx, cands, &choice, &active, seam);
        (choice, w)
    };
    // Moves measured apart can interact where they meet, so the few best
    // plans are scored on the mesh and the best of those is kept.
    let real = |w: &Weights, gate_first: bool| -> (i64, f64) {
        let ev = evaluate_with(model, ctx, w, false);
        let re = region_energy(ctx, &ev);
        let mut p = problem(Vec::new(), 0.0);
        p.gate_first = gate_first;
        let change: f64 = (0..model.nverts()).map(|v| l1(&w[v], &model.weights[v])).sum();
        (p.fails(&flag_counts(ctx, &ev.flags, None)), crate::pick::energy_of(&size, &re) + p.edit_cost * change)
    };
    let choose = |cands: &[Candidate], p: &crate::pick::Problem, tag: &str, from: &[usize]| -> (Vec<usize>, Weights, (i64, f64), bool) {
        let plans = crate::pick::pick_top(p, 4);
        let mut best: Option<(Vec<usize>, Weights, (i64, f64))> = None;
        for plan in &plans {
            let (c, wt) = apply(cands, p, plan, from);
            let r = real(&wt, p.gate_first);
            if debug {
                eprintln!(
                    "{tag}: {} moves, predicted ({}, {:.4}), real ({}, {:.4}), exact {}",
                    plan.moves.len(),
                    plan.fails,
                    plan.energy,
                    r.0,
                    r.1,
                    plan.exact
                );
            }
            if best.as_ref().is_none_or(|b| r.0 < b.2.0 || (r.0 == b.2.0 && r.1 < b.2.1 - 1e-12)) {
                best = Some((c, wt, r));
            }
        }
        let (c, w, r) = best.unwrap();
        (c, w, r, plans[0].exact)
    };
    // A candidate is measured only if, applied to the whole repair area, it
    // improves some failing region: the rest cannot repair anything, and
    // measuring costs one mesh evaluation per region set. Depends only on
    // the candidate's own numbers, so a better candidate is never dropped.
    let useful = |c: &Candidate| (0..nr).any(|r| failing[r] && c.region_e[r] < orig_e[r] - 1e-9);
    let mut moves: Vec<crate::pick::Move> =
        (1..cands.len()).filter(|&c| useful(&cands[c])).flat_map(|c| measure(&cands, c, &input_base)).collect();
    let p1 = problem(moves.clone(), 0.01);
    let (mut choice, mut w, mut best_real, mut exact) = choose(&cands, &p1, "pick 1", &input_base.choice);
    let mut groups: Vec<(&Ctx, bool)> = vec![(ctx, false)];
    if let Some(m) = o.motion {
        groups.push((m, true));
    }
    let area = crate::owner::vertex_areas(model);
    let total_area: f64 = area.iter().sum::<f64>().max(1e-12);
    let tears_pct = |wt: &Weights| -> Vec<f64> {
        groups
            .iter()
            .map(|&(c, m)| {
                let t = crate::owner::torn(model, c, wt, m);
                (0..model.nverts()).filter(|&v| t[v]).map(|v| area[v]).sum::<f64>() / total_area * 100.0
            })
            .collect()
    };
    let tin_pct = if o.headline_guard { tears_pct(&model.weights) } else { Vec::new() };
    // Every stage's result is judged on the mesh; the fix is the best one
    // that breaks no rule (matching `compare --zoom`: gate first, then tears
    // when the headline judges, then score), so a later stage never undoes
    // an earlier good one. Hard rule: no region may gain a failing finding
    // it did not have.
    let new_fails = |rep: &Report| {
        rep.findings.iter().any(|f| f.severity == "fail" && !rep0.findings.iter().any(|g| g.severity == "fail" && g.region == f.region))
    };
    type Kept = (Vec<usize>, Weights, Eval, Report, Vec<f64>);
    let mut kept: Option<Kept> = None;
    let consider = |kept: &mut Option<Kept>, choice: &[usize], w: &Weights| -> bool {
        let ev = evaluate(model, ctx, w);
        let rep = build(file, model, ctx, &ev);
        if std::env::var_os("WF_DEBUG").is_some() {
            let gained: Vec<String> = rep
                .findings
                .iter()
                .filter(|f| f.severity == "fail" && !rep0.findings.iter().any(|g| g.severity == "fail" && g.region == f.region))
                .map(|f| format!("{} {} {}", f.region, f.code, f.verts))
                .collect();
            eprintln!("stage: score {:.1}, pass {}, gains {:?}", rep.score, rep.pass, gained);
        }
        if new_fails(&rep) {
            return false;
        }
        let tp = if o.headline_guard { tears_pct(w) } else { Vec::new() };
        let beats = |k: &Kept| -> bool {
            if rep.pass != k.3.pass {
                return rep.pass;
            }
            if o.headline_guard {
                let worse_new = tp.iter().zip(&tin_pct).any(|(a, b)| *a > *b + 1e-9);
                let worse_old = k.4.iter().zip(&tin_pct).any(|(a, b)| *a > *b + 1e-9);
                if worse_new != worse_old {
                    return !worse_new;
                }
                let sa: f64 = tp.iter().sum();
                let sb: f64 = k.4.iter().sum();
                if (sa - sb).abs() > 0.05 * groups.len() as f64 {
                    return sa < sb;
                }
            }
            rep.score > k.3.score
        };
        if kept.as_ref().is_none_or(beats) {
            *kept = Some((choice.to_vec(), w.clone(), ev, rep, tp));
        }
        true
    };
    let mut valid = consider(&mut kept, &choice, &w);

    // Method 4 starts from the best mixes so far: the plan picked for the
    // gate and the plan with the lowest score energy (it often clears the
    // last findings from there), then everything is picked again with them
    // as more candidates (the earlier plans stay possible).
    if o.methods.contains(&Method::Optimize) {
        let mut seeds = vec![w.clone()];
        // When nothing passes the gate, the score-only pick is the same
        // search; run it only when the gate pick passes.
        if best_real.0 == 0 {
            let mut by_energy = problem(moves.clone(), 0.01);
            by_energy.gate_first = false;
            let (_, we, _, e) = choose(&cands, &by_energy, "pick 1 (score only)", &input_base.choice);
            exact &= e;
            if we != w {
                seeds.push(we);
            }
        }
        for (k, seed) in seeds.iter().enumerate() {
            let (ev_mix, _) = score(seed);
            let opt = optimize(model, ctx, seed, &ev_mix);
            push(if k == 0 { "optimize".into() } else { "optimize(score)".into() }, opt, &mut cands);
            if useful(&cands[cands.len() - 1]) {
                moves.extend(measure(&cands, cands.len() - 1, &input_base));
            }
        }
        let p2 = problem(moves.clone(), 0.01);
        let (c2, w2, r2, e2) = choose(&cands, &p2, "pick 2", &input_base.choice);
        exact &= e2;
        let v2 = consider(&mut kept, &c2, &w2);
        if (v2 && !valid) || (v2 == valid && (r2.0 < best_real.0 || (r2.0 == best_real.0 && r2.1 < best_real.1 - 1e-12))) {
            choice = c2;
            w = w2;
            best_real = r2;
            valid = v2;
        }
    }

    // Refinement: moves measured apart miss how neighboring repairs meet
    // (seams between two candidates). Re-measure every move against the
    // current plan (including putting regions back to the input) and pick
    // again, exactly; go on while the result keeps the rules and scores
    // better on the mesh, or repairs a rule the current plan broke. Rules
    // stay relative to the input.
    for round in 0..4 {
        let base = base_of(choice.clone(), w.clone());
        let moves_r: Vec<crate::pick::Move> = (0..cands.len())
            .filter(|&c| c == 0 || useful(&cands[c]) || base.choice.contains(&c))
            .flat_map(|c| measure(&cands, c, &base))
            .collect();
        let mut p = problem_at(&base, moves_r, 0.01);
        p.min_gain = 0.0;
        let (c3, w3, r3, e3) = choose(&cands, &p, &format!("refine {round}"), &base.choice);
        if c3 == choice {
            break;
        }
        let v3 = consider(&mut kept, &c3, &w3);
        let gain = r3.0 < best_real.0 || (r3.0 == best_real.0 && r3.1 < best_real.1 - 1e-9);
        // Follow a better-scoring plan even if it breaks a rule (the next
        // round can repair it); the kept result only ever takes plans that
        // keep the rules.
        if !(gain || (v3 && !valid)) {
            break;
        }
        exact &= e3;
        choice = c3;
        w = w3;
        best_real = r3;
        valid = v3;
    }

    // The last plan scores best on the mesh but breaks a rule: measured
    // apart, its repairs interact (a region between two repairs gains a
    // finding). Drop repairs one at a time, each time the one whose removal
    // leaves the fewest such regions and then the lowest energy, and offer
    // the first rule-abiding result. A repair is a connected set of regions
    // on one candidate.
    let count_new = |rep: &Report| -> usize {
        let mut regions: Vec<&str> = rep
            .findings
            .iter()
            .filter(|f| f.severity == "fail" && !rep0.findings.iter().any(|g| g.severity == "fail" && g.region == f.region))
            .map(|f| f.region.as_str())
            .collect();
        regions.sort_unstable();
        regions.dedup();
        regions.len()
    };
    if new_fails(&build(file, model, ctx, &evaluate(model, ctx, &w))) {
        let mut cur = choice.clone();
        loop {
            let mut units: Vec<Vec<usize>> = Vec::new();
            let mut seen = vec![false; nr];
            for r in 0..nr {
                if cur[r] == 0 || seen[r] {
                    continue;
                }
                let mut unit = vec![r];
                seen[r] = true;
                let mut i = 0;
                while i < unit.len() {
                    for &k in &adjr[unit[i]] {
                        if !seen[k] && cur[k] == cur[r] {
                            seen[k] = true;
                            unit.push(k);
                        }
                    }
                    i += 1;
                }
                units.push(unit);
            }
            if units.is_empty() {
                break;
            }
            let mut best: Option<((usize, f64), Vec<usize>, Weights)> = None;
            for unit in &units {
                let mut trial = cur.clone();
                for &k in unit {
                    trial[k] = 0;
                }
                let wt = blend(model, ctx, &cands, &trial, &active, seam);
                let ev_t = evaluate_with(model, ctx, &wt, false);
                let rep_t = build(file, model, ctx, &ev_t);
                let key = (count_new(&rep_t), crate::pick::energy_of(&size, &region_energy(ctx, &ev_t)));
                if best.as_ref().is_none_or(|b| key.0 < b.0.0 || (key.0 == b.0.0 && key.1 < b.0.1)) {
                    best = Some((key, trial, wt));
                }
            }
            let (key, trial, wt) = best.unwrap();
            if debug {
                eprintln!("drop: {} regions gain a failing finding, energy {:.4}", key.0, key.1);
            }
            cur = trial;
            if key.0 == 0 {
                consider(&mut kept, &cur, &wt);
                break;
            }
        }
    }
    let (mut choice, mut w, mut ev, mut rep1) = match kept {
        Some((c, w, ev, rep, _)) => (c, w, ev, rep),
        None => (vec![0; nr], model.weights.clone(), evaluate(model, ctx, &model.weights), rep0.clone()),
    };
    // Headline-only candidates: copying one side onto the other or routing
    // other-leg weight to the pelvis repairs lopsided generated crotches
    // where they actually reduce torn skin, without letting the continuous
    // ROM energy search overwrite symmetric healthy limbs (CesiumMan, MPFB).
    if o.headline_guard && o.methods.contains(&Method::Smooth) {
        if let Some(wc) = crotch_to_pelvis(model) {
            push("crotch-to-pelvis".into(), wc, &mut cands);
        }
        for (name, from_left) in [("mirror-left", true), ("mirror-right", false)] {
            if let Some(wm) = mirror(model, from_left) {
                push(name.into(), wm, &mut cands);
            }
        }
    }
    // Headline choice: a failing region that still tears takes the candidate
    // that tears least there, if that is clearly less (2 cm^2 and a tenth)
    // AND obeys the same deformation rules as `pick` and `compare --zoom`:
    // never turning a passing rig into a failing one, never adding a new
    // failing finding, and never pushing a region past its energy cap (on
    // fused robes and crotches, an uncapped tear override trades wrong-bone
    // drag for 2.0-3.4x flat tent-sheet stretching across the legs). Among
    // candidates with similar tears, smaller weight changes from the input
    // win so repairs stay closer to professional rigs.
    if o.headline_guard && cands.len() > 1 {
        let k = crate::owner::BODY_M / crate::owner::height(model);
        // Torn area per region, per group (poses, clips), in cm^2.
        let torn_area = |wt: &Weights| -> Vec<Vec<f64>> {
            groups
                .iter()
                .map(|&(c, m)| {
                    let t = crate::owner::torn(model, c, wt, m);
                    let mut a = vec![0.0; nr];
                    for v in (0..model.nverts()).filter(|&v| t[v]) {
                        a[ctx.vregion[v] as usize] += area[v] * k * k * 1e4;
                    }
                    a
                })
                .collect()
        };
        let sum_r = |a: &[Vec<f64>], r: usize| a.iter().map(|g| g[r]).sum::<f64>();
        let total = |a: &[Vec<f64>], g: usize| a[g].iter().sum::<f64>();
        let cur_a = torn_area(&w);
        if (0..nr).any(|r| failing[r] && sum_r(&cur_a, r) > 0.0) {
            let cur_re = region_energy(ctx, &ev);
            let cap_r = |r: usize| orig_e[r].max(cur_re[r]) * 1.1 + 0.01;
            let cand_l1: Vec<Vec<f64>> = cands
                .iter()
                .map(|c| {
                    let mut s = vec![0.0; nr];
                    for v in 0..model.nverts() {
                        s[ctx.vregion[v] as usize] += 0.5 * l1(&c.w[v], &model.weights[v]);
                    }
                    for r in 0..nr {
                        s[r] /= size[r].max(1.0);
                    }
                    s
                })
                .collect();
            const STAY_NEAR_CM2: f64 = 15.0;
            let cand_a: Vec<Vec<Vec<f64>>> = cands.iter().map(|c| torn_area(&c.w)).collect();
            let tin = &cand_a[0];
            debug_assert_eq!(cands[0].name, "original");
            let worse_than_input = |a: &[Vec<f64>]| (0..groups.len()).any(|g| total(a, g) > total(tin, g) + 1e-9);
            let must_pass = rep0.pass || (rep1.pass && !worse_than_input(&cur_a));
            let mut with_cur = vec![Candidate { name: "current".into(), w: w.clone(), region_e: Vec::new() }];
            with_cur.extend(cands.iter().map(|c| Candidate { name: c.name.clone(), w: c.w.clone(), region_e: Vec::new() }));
            let all = vec![true; model.nverts()];
            let original = 1;
            let sum_all = |a: &[Vec<f64>]| (0..groups.len()).map(|g| total(a, g)).sum::<f64>();
            let choose = |strict: bool| -> Option<(Vec<usize>, Weights, Vec<Vec<f64>>)> {
                let mut pick = vec![0usize; nr];
                for r in 0..nr {
                    let cur = sum_r(&cur_a, r);
                    if !failing[r] || cur <= 0.0 {
                        continue;
                    }
                    let best = cand_a
                        .iter()
                        .enumerate()
                        .filter(|&(i, a)| {
                            cands[i].region_e[r] <= cap_r(r) + 1e-9 && (!strict || (0..groups.len()).all(|g| a[g][r] <= cur_a[g][r] + 1e-9))
                        })
                        .map(|(i, a)| (i, sum_r(a, r) + STAY_NEAR_CM2 * cand_l1[i][r]))
                        .fold((usize::MAX, f64::MAX), |b, x| if x.1 < b.1 { x } else { b });
                    if best.0 != usize::MAX && sum_r(&cand_a[best.0], r) + (0.1 * cur).max(2.0) < cur {
                        pick[r] = best.0 + 1;
                    }
                }
                for _ in 0..=nr {
                    if !pick.iter().any(|&p| p > 0) {
                        return None;
                    }
                    let w2 = blend(model, ctx, &with_cur, &pick, &all, seam);
                    let after = torn_area(&w2);
                    if let Some(g) = (0..groups.len()).find(|&g| total(&after, g) > total(tin, g) + 1e-9) {
                        let back = (0..nr)
                            .filter(|&r| pick[r] != original && after[g][r] > tin[g][r])
                            .max_by(|&a, &b| (after[g][a] - tin[g][a]).total_cmp(&(after[g][b] - tin[g][b])).then(b.cmp(&a)));
                        let r = back?;
                        pick[r] = original;
                        continue;
                    }
                    let ev2 = evaluate_with(model, ctx, &w2, false);
                    let rep2 = build(file, model, ctx, &ev2);
                    let re2 = region_energy(ctx, &ev2);
                    if new_fails(&rep2) || (must_pass && !rep2.pass) || (0..nr).any(|r| re2[r] > cap_r(r) + 1e-9) {
                        let fallback = if worse_than_input(&cur_a) { original } else { 0 };
                        let back = (0..nr)
                            .filter(|&r| pick[r] > 0 && pick[r] != fallback)
                            .max_by(|&a, &b| (re2[a] - cap_r(a)).total_cmp(&(re2[b] - cap_r(b))).then(b.cmp(&a)));
                        let r = back?;
                        pick[r] = fallback;
                        continue;
                    }
                    return (worse_than_input(&cur_a) || sum_all(&after) + 2.0 < sum_all(&cur_a)).then_some((pick, w2, after));
                }
                None
            };
            let best = [choose(true), choose(false)].into_iter().flatten().min_by(|a, b| sum_all(&a.2).total_cmp(&sum_all(&b.2)));
            if let Some((pick, w2, after)) = best {
                if std::env::var_os("WF_DEBUG").is_some() {
                    eprintln!(
                        "headline choice totals: input {:?}, pick {:?}, choice {:?}",
                        (0..groups.len()).map(|g| total(tin, g)).collect::<Vec<_>>(),
                        (0..groups.len()).map(|g| total(&cur_a, g)).collect::<Vec<_>>(),
                        (0..groups.len()).map(|g| total(&after, g)).collect::<Vec<_>>()
                    );
                    let names: Vec<String> = (0..nr)
                        .filter(|&r| pick[r] > 0)
                        .map(|r| format!("{} <- {}", ctx.regions[r].label, cands[pick[r] - 1].name))
                        .collect();
                    eprintln!("headline choice: {}", names.join(", "));
                }
                for r in 0..nr {
                    if pick[r] > 0 {
                        choice[r] = pick[r] - 1;
                    }
                }
                w = w2;
                ev = evaluate(model, ctx, &w);
                rep1 = build(file, model, ctx, &ev);
            }
        }
    }
    if o.headline_guard && w != model.weights {
        let (wg, back) = crate::owner::guard(model, &groups, &model.weights, &w);
        if wg != w {
            if std::env::var_os("WF_DEBUG").is_some() {
                eprintln!("headline guard: input weights back on {}", back.join(", "));
            }
            for (r, ch) in choice.iter_mut().enumerate().take(nr) {
                if (0..model.nverts()).any(|v| ctx.vregion[v] as usize == r && wg[v] != w[v]) {
                    *ch = 0;
                }
            }
            w = wg;
            ev = evaluate(model, ctx, &w);
            rep1 = build(file, model, ctx, &ev);
        }
    }
    // The score's gate, when the headline does not judge: with the guard
    // on, nothing is kept that tears more skin than the input, and the
    // score may disagree with what is seen (it did on Tripo's crotch).
    if !o.headline_guard && (ranks_below(&rep1, &rep0) || new_fails(&rep1)) {
        w = model.weights.clone();
        ev = ev0;
        rep1 = build(file, model, ctx, &ev);
        choice = vec![0; nr];
    }
    // Never worse than the input on what the check reports: if the result
    // tears more skin than the input on the clips or on the poses (any
    // amount), the input comes back (character line round 3: a fix kept a
    // worse rig, 2026-11-14).
    let mut tears = (Vec::new(), Vec::new());
    if o.headline_guard {
        let mut tout = if w == model.weights { tin_pct.clone() } else { tears_pct(&w) };
        if tout.iter().zip(&tin_pct).any(|(a, b)| *a > *b + 1e-9) {
            if std::env::var_os("WF_DEBUG").is_some() {
                eprintln!("never worse: tears {tout:?} against the input's {tin_pct:?}; the input comes back");
            }
            w = model.weights.clone();
            ev = evaluate(model, ctx, &w);
            rep1 = build(file, model, ctx, &ev);
            choice = vec![0; nr];
            tout = tin_pct.clone();
        }
        tears = (tin_pct, tout);
    }
    let names: Vec<String> = groups.iter().map(|&(_, m)| if m { "game clips".to_string() } else { "poses".to_string() }).collect();
    let torn_cm2 = o.headline_guard.then(|| {
        let k = crate::owner::BODY_M / crate::owner::height(model);
        tears.1.iter().sum::<f64>() / 100.0 * total_area * k * k * 1e4
    });

    let re1 = region_energy(ctx, &ev);
    let regions = (0..nr)
        .filter(|&r| failing[r] || choice[r] != 0)
        .map(|r| RegionChoice {
            name: ctx.regions[r].name.clone(),
            label: ctx.regions[r].label.clone(),
            before: crate::report::energy_to_score(orig_e[r]),
            after: crate::report::energy_to_score(re1[r]),
            chosen: cands[choice[r]].name.clone(),
            candidates: cands.iter().map(|c| (c.name.clone(), crate::report::energy_to_score(c.region_e[r]))).collect(),
        })
        .collect();
    let changes: Vec<f64> = (0..model.nverts()).map(|v| l1(&w[v], &model.weights[v])).collect();
    let fix = FixReport {
        tool: "weightforge".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        input: file.into(),
        methods: o.methods.iter().map(|m| m.name().to_string()).chain(o.external.iter().map(|e| format!("external:{}", e.0))).collect(),
        before: summary(&rep0),
        after: summary(&rep1),
        improved: if tears.0.is_empty() {
            rep1.score > rep0.score
        } else {
            tears.1.iter().sum::<f64>() + 1e-9 < tears.0.iter().sum::<f64>()
        },
        pick_exact: exact,
        external_kept: !o.external.is_empty(),
        verts_changed: changes.iter().filter(|&&c| c > 1e-4).count(),
        mean_l1_change: (changes.iter().sum::<f64>() / model.nverts().max(1) as f64 * 1e6).round() / 1e6,
        max_influences: w.iter().map(Vec::len).max().unwrap_or(0),
        candidates: cands
            .iter()
            .map(|c| {
                let mean: f64 = c.region_e.iter().sum::<f64>() / nr.max(1) as f64;
                (c.name.clone(), crate::report::energy_to_score(mean))
            })
            .collect(),
        regions,
        findings_before: rep0.findings.clone(),
        findings_after: rep1.findings.clone(),
        tears_before: names.iter().cloned().zip(tears.0.iter().map(|x| (x * 100.0).round() / 100.0 + 0.0)).collect(),
        tears_after: names.iter().cloned().zip(tears.1.iter().map(|x| (x * 100.0).round() / 100.0 + 0.0)).collect(),
    };
    FixResult { weights: w, eval: ev, report_before: rep0, report_after: rep1, fix, torn_cm2 }
}

/// Whether a result ranks below another by the order every stage of the
/// fix keeps: passing the gate first, then score. The last guard reverts to
/// the input on this same order (it used to revert on score alone, so a
/// result that passed at a lower score than a failing input was thrown away
/// after the pick had kept it for passing).
pub fn ranks_below(a: &Report, b: &Report) -> bool {
    (a.pass, a.score) < (b.pass, b.score)
}

/// Mean L1 distance (halved, so 0..1 per vertex) between two weight sets.
pub fn weight_error(a: &Weights, b: &Weights) -> f64 {
    a.iter().zip(b).map(|(x, y)| 0.5 * l1(x, y)).sum::<f64>() / a.len().max(1) as f64
}
