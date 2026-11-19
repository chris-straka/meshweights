use crate::metrics::Ctx;
use crate::report::Report;
use crate::scene::{Model, VW, Weights, normalize_vw, prune_vw, weight_of};
use rayon::prelude::*;

use super::Candidate;
use super::Summary;
use super::helpers::distances;

// ---------------------------------------------------------------- picking

pub(super) fn l1(a: &VW, b: &VW) -> f64 {
    a.iter().map(|&(j, x)| (x - weight_of(b, j)).abs()).sum::<f64>()
        + b.iter().filter(|e| weight_of(a, e.0) == 0.0).map(|e| e.1).sum::<f64>()
}

/// Standard normal CDF (Abramowitz and Stegun 7.1.26, error < 2e-7).
fn phi(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * z);
    let poly = t * (0.254829592 + t * (-0.284496736 + t * (1.421413741 + t * (-1.453152027 + t * 1.061405429))));
    let erf = 1.0 - poly * (-z * z).exp();
    if x >= 0.0 { 0.5 * (1.0 + erf) } else { 0.5 * (1.0 - erf) }
}

/// Blends per-region candidate choices across their seams like a Gaussian
/// blur of width `sigma` (metres), at any mesh density: a vertex keeps
/// `Phi(s / sigma)` of its own candidate, `s` being its rest-pose edge-path
/// distance to the nearest vertex assigned otherwise (less half an edge,
/// where the seam runs), and shares the rest among those other candidates
/// by the same rule. One bounded Dijkstra per candidate in use; vertices
/// more than 3 sigma from every seam keep their candidate's weights exactly.
pub(super) fn blend(model: &Model, ctx: &Ctx, cands: &[Candidate], choice: &[usize], active: &[bool], sigma: f64) -> Weights {
    let nv = model.nverts();
    let assign: Vec<usize> = (0..nv).map(|v| if active[v] { choice[ctx.vregion[v] as usize] } else { 0 }).collect();
    let mut used: Vec<usize> = assign.clone();
    used.sort_unstable();
    used.dedup();
    if used == [0] {
        return cands[0].w.clone();
    }
    let reach = 3.0 * sigma;
    let dist: Vec<Vec<f64>> =
        used.par_iter().map(|&c| distances(model, &assign.iter().map(|&a| a == c).collect::<Vec<_>>(), reach)).collect();
    (0..nv)
        .into_par_iter()
        .map(|v| {
            let own = assign[v];
            let half = 0.5 * model.adj[v].iter().map(|&u| (model.rest[u as usize] - model.rest[v]).len()).fold(f64::MAX, f64::min);
            let half = if half.is_finite() { half } else { 0.0 };
            // Other candidates within reach and how much of each spills here.
            let mut others: Vec<(usize, f64)> = Vec::new();
            let mut nearest = f64::INFINITY;
            for (k, &c) in used.iter().enumerate() {
                let d = dist[k][v];
                if c != own && d.is_finite() {
                    nearest = nearest.min(d);
                    let x = if sigma > 0.0 { 1.0 - phi((d - half).max(0.0) / sigma) } else { 0.0 };
                    if x > 1e-6 {
                        others.push((c, x));
                    }
                }
            }
            if others.is_empty() {
                return cands[own].w[v].clone(); // away from seams: exact
            }
            let keep = if sigma > 0.0 { phi((nearest - half).max(0.0) / sigma) } else { 1.0 };
            let total: f64 = others.iter().map(|o| o.1).sum();
            let mut acc: VW = cands[own].w[v].iter().map(|&(j, y)| (j, y * keep)).collect();
            for &(c, x) in &others {
                acc.extend(cands[c].w[v].iter().map(|&(j, y)| (j, y * (1.0 - keep) * x / total)));
            }
            prune_vw(&normalize_vw(acc), 4, 0.01)
        })
        .collect()
}

/// Flagged-vertex counts per region and flag (`pick::NFLAG` slots in
/// `FLAG_NAMES` order; self-intersection, a warning, is not counted).
pub(super) fn flag_counts(ctx: &Ctx, flags: &[u8], regions: Option<&[bool]>) -> Vec<[i64; crate::pick::NFLAG]> {
    let mut out = vec![[0i64; crate::pick::NFLAG]; ctx.regions.len()];
    for (v, &f) in flags.iter().enumerate() {
        let r = ctx.vregion[v] as usize;
        if f == 0 || regions.is_some_and(|d| !d[r]) {
            continue;
        }
        for (i, (bit, _)) in crate::metrics::FLAG_NAMES.iter().enumerate() {
            if f & bit != 0 && *bit != crate::metrics::F_INTERSECT {
                out[r][i] += 1;
            }
        }
    }
    out
}

/// Per region and flag: the flagged count at which `report::build` makes
/// it a failing finding, and the highest count allowed (one less, for
/// regions that do not fail yet; unlimited where the region already fails).
pub(super) type Counts = Vec<[i64; crate::pick::NFLAG]>;
pub(super) fn fail_limits(model: &Model, ctx: &Ctx, had_fail: &[bool]) -> (Counts, Counts) {
    use crate::metrics::{F_FOLLOW, F_INTERSECT, F_PIECE, F_UNWEIGHTED, FLAG_NAMES};
    let mut attached = vec![0i64; ctx.regions.len()];
    for m in ctx.piece_matches.iter().filter(|m| m.3) {
        attached[ctx.vregion[m.0 as usize] as usize] += 1;
    }
    let _ = model;
    let fail_at: Counts = (0..ctx.regions.len())
        .map(|r| {
            std::array::from_fn(|i| {
                let bit = FLAG_NAMES[i].0;
                let min = if bit == F_UNWEIGHTED || bit == F_PIECE { 1 } else { ctx.th.region_min_bad as i64 };
                match bit {
                    F_INTERSECT => i64::MAX,
                    // A piece straying in fewer than half its attached verts is a warning.
                    F_FOLLOW => min.max((attached[r] + 1) / 2),
                    _ => min,
                }
            })
        })
        .collect();
    let limit = (0..ctx.regions.len())
        .map(|r| if had_fail[r] { [i64::MAX; crate::pick::NFLAG] } else { fail_at[r].map(|x| if x == i64::MAX { x } else { x - 1 }) })
        .collect();
    (fail_at, limit)
}

/// Edit cost: score energy per unit of mean weight change (`pick::Problem::
/// edit_cost`). 0.04 weighs a plan that changes every vertex's weights by
/// 0.05 (L1) like 0.002 of energy: joint falloff repairs clear it, while
/// sub-point energy noise on healthy neighbours leaves the input alone.
pub(super) const EDIT_MU: f64 = 0.04;

pub(super) fn summary(r: &Report) -> Summary {
    Summary { score: r.score, pass: r.pass, fails: r.findings.iter().filter(|f| f.severity == "fail").count() }
}
