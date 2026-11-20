//! Cut-cell geodesic graph in place of the voxel graph (meshweights-7):
//!
//! 1. On a single watertight mesh with a narrow (5 mm) air gap between two
//!    thighs, geodesic distances must route up through the pelvis instead of
//!    bridging across the gap, and `fix::geodesic` must not bleed right-thigh
//!    weight onto the left inner thigh (or vice versa).
//! 2. On a single watertight torso + shoulder + hanging arm with a 5 mm air
//!    gap between torso and inner arm/hand, geodesic distances from the spine
//!    to the inner forearm/hand must route up through the shoulder instead of
//!    jumping the gap, and `fix::geodesic` must not bind the arm to the spine.
//! 3. Overlapping shells (like the mannequin's arm/leg tubes penetrating the
//!    torso) must remain connected through the union of shells.
//! 4. Thin parts narrower than a voxel cell must remain reachable (finite
//!    geodesic distance < cap), and geodesic outputs must be bit-deterministic.

use std::collections::BTreeMap;
use weightforge_core::fix;
use weightforge_core::fixtures::{self, Bone, Fixture, to_glb};
use weightforge_core::math::{Vec3, v3};
use weightforge_core::metrics::{Ctx, CtxOpts};
use weightforge_core::scene::{Model, VW, weight_of};
use weightforge_core::voxel::geodesics;

/// Builds a watertight, manifold, outward-oriented triangle mesh for the union
/// of axis-aligned boxes `[min, max]`, subdivided so no edge exceeds `step`.
fn box_union_mesh(boxes: &[([f64; 3], [f64; 3])], step: f64) -> (Vec<Vec3>, Vec<[u32; 3]>) {
    let mut axes = [Vec::<f64>::new(), Vec::<f64>::new(), Vec::<f64>::new()];
    for &(lo, hi) in boxes {
        for a in 0..3 {
            axes[a].push(lo[a]);
            axes[a].push(hi[a]);
        }
    }
    let mut grids: [Vec<f64>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for a in 0..3 {
        axes[a].sort_by(|x, y| x.total_cmp(y));
        axes[a].dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        let mut g = Vec::new();
        for w in axes[a].windows(2) {
            let len = w[1] - w[0];
            let n = (len / step).ceil().max(1.0) as usize;
            for i in 0..n {
                g.push(w[0] + len * (i as f64 / n as f64));
            }
        }
        g.push(*axes[a].last().unwrap());
        grids[a] = g;
    }

    let (nx, ny, nz) = (grids[0].len() - 1, grids[1].len() - 1, grids[2].len() - 1);
    let mut occ = vec![false; nx * ny * nz];
    let cidx = |ix: usize, iy: usize, iz: usize| (iz * ny + iy) * nx + ix;
    for iz in 0..nz {
        let mz = 0.5 * (grids[2][iz] + grids[2][iz + 1]);
        for iy in 0..ny {
            let my = 0.5 * (grids[1][iy] + grids[1][iy + 1]);
            for ix in 0..nx {
                let mx = 0.5 * (grids[0][ix] + grids[0][ix + 1]);
                let inside = boxes.iter().any(|&(lo, hi)| mx > lo[0] && mx < hi[0] && my > lo[1] && my < hi[1] && mz > lo[2] && mz < hi[2]);
                occ[cidx(ix, iy, iz)] = inside;
            }
        }
    }

    let mut pos: Vec<Vec3> = Vec::new();
    let mut vmap: BTreeMap<(usize, usize, usize), u32> = BTreeMap::new();
    let mut vert = |ix: usize, iy: usize, iz: usize| -> u32 {
        *vmap.entry((ix, iy, iz)).or_insert_with(|| {
            let id = pos.len() as u32;
            pos.push(v3(grids[0][ix], grids[1][iy], grids[2][iz]));
            id
        })
    };

    let mut tris: Vec<[u32; 3]> = Vec::new();
    let is_occ = |ix: isize, iy: isize, iz: isize| -> bool {
        if ix < 0 || iy < 0 || iz < 0 || ix as usize >= nx || iy as usize >= ny || iz as usize >= nz {
            return false;
        }
        occ[cidx(ix as usize, iy as usize, iz as usize)]
    };

    for iz in 0..nz {
        for iy in 0..ny {
            for ix in 0..nx {
                if !occ[cidx(ix, iy, iz)] {
                    continue;
                }
                let (x, y, z) = (ix as isize, iy as isize, iz as isize);
                // +X
                if !is_occ(x + 1, y, z) {
                    let (a, b, c, d) =
                        (vert(ix + 1, iy, iz), vert(ix + 1, iy + 1, iz), vert(ix + 1, iy + 1, iz + 1), vert(ix + 1, iy, iz + 1));
                    tris.push([a, b, c]);
                    tris.push([a, c, d]);
                }
                // -X
                if !is_occ(x - 1, y, z) {
                    let (a, b, c, d) = (vert(ix, iy, iz), vert(ix, iy + 1, iz), vert(ix, iy + 1, iz + 1), vert(ix, iy, iz + 1));
                    tris.push([a, c, b]);
                    tris.push([a, d, c]);
                }
                // +Y
                if !is_occ(x, y + 1, z) {
                    let (a, b, c, d) =
                        (vert(ix, iy + 1, iz), vert(ix, iy + 1, iz + 1), vert(ix + 1, iy + 1, iz + 1), vert(ix + 1, iy + 1, iz));
                    tris.push([a, b, c]);
                    tris.push([a, c, d]);
                }
                // -Y
                if !is_occ(x, y - 1, z) {
                    let (a, b, c, d) = (vert(ix, iy, iz), vert(ix, iy, iz + 1), vert(ix + 1, iy, iz + 1), vert(ix + 1, iy, iz));
                    tris.push([a, c, b]);
                    tris.push([a, d, c]);
                }
                // +Z
                if !is_occ(x, y, z + 1) {
                    let (a, b, c, d) =
                        (vert(ix, iy, iz + 1), vert(ix + 1, iy, iz + 1), vert(ix + 1, iy + 1, iz + 1), vert(ix, iy + 1, iz + 1));
                    tris.push([a, b, c]);
                    tris.push([a, c, d]);
                }
                // -Z
                if !is_occ(x, y, z - 1) {
                    let (a, b, c, d) = (vert(ix, iy, iz), vert(ix + 1, iy, iz), vert(ix + 1, iy + 1, iz), vert(ix, iy + 1, iz));
                    tris.push([a, c, b]);
                    tris.push([a, d, c]);
                }
            }
        }
    }
    (pos, tris)
}

#[test]
fn narrow_gap_single_shell_crotch_does_not_bridge() {
    // Single watertight U-arch: pelvis at y in [0.80, 1.00], two thighs down to y = 0.10
    // separated by a 5 mm air gap (x in [-0.0025, +0.0025]).
    // At res = 64, cell size h ~ 0.90 / 64 = 0.014 m (~2.8x wider than the 5 mm gap).
    let (pos, tris) = box_union_mesh(
        &[
            ([-0.1425, 0.80, -0.07], [0.1425, 1.00, 0.07]),
            ([0.0025, 0.10, -0.07], [0.1425, 0.80, 0.07]),
            ([-0.1425, 0.10, -0.07], [-0.0025, 0.80, 0.07]),
        ],
        0.025,
    );
    // Add child tip joints (`knee.L`, `knee.R`) so glTF preserves the exact tail
    // of `thigh.L` and `thigh.R` rather than estimating short leaf stubs.
    let bones = vec![
        Bone { name: "pelvis", parent: None, head: v3(0.0, 0.84, 0.0), tail: v3(0.0, 0.97, 0.0) },
        Bone { name: "thigh.L", parent: Some(0), head: v3(0.0725, 0.80, 0.0), tail: v3(0.0725, 0.15, 0.0) },
        Bone { name: "thigh.R", parent: Some(0), head: v3(-0.0725, 0.80, 0.0), tail: v3(-0.0725, 0.15, 0.0) },
        Bone { name: "knee.L", parent: Some(1), head: v3(0.0725, 0.15, 0.0), tail: v3(0.0725, 0.11, 0.0) },
        Bone { name: "knee.R", parent: Some(2), head: v3(-0.0725, 0.15, 0.0), tail: v3(-0.0725, 0.11, 0.0) },
    ];
    let body_w: Vec<VW> = pos
        .iter()
        .map(|p| {
            let j = if p.y >= 0.80 {
                0
            } else if p.x >= 0.0 {
                1
            } else {
                2
            };
            vec![(j, 1.0)]
        })
        .collect();
    let f = Fixture { bones, body_pos: pos, body_tris: tris, cape_pos: Vec::new(), cape_tris: Vec::new(), body_w, cape_w: Vec::new() };
    let m = Model::from_glb(to_glb(&f)).unwrap();
    assert!((m.skel.tail[1] - v3(0.0725, 0.15, 0.0)).len() < 1e-6, "thigh.L tail must reach y=0.15");
    assert!((m.skel.tail[2] - v3(-0.0725, 0.15, 0.0)).len() < 1e-6, "thigh.R tail must reach y=0.15");

    let geo = geodesics(&m, 64, &[0, 1, 2]);
    assert_eq!(geo.solid.openings, 0, "crotch test mesh must be watertight");

    // Check inner-thigh vertices well below the crotch (y in [0.25, 0.55]).
    // True path from inner face to the opposite thigh bone must climb to the
    // crotch (y = 0.80) and cross ~0.075 m over the pelvis: >= (0.80 - y) + 0.05.
    let mut checked_l = 0usize;
    let mut checked_r = 0usize;
    for (v, p) in m.rest.iter().enumerate() {
        if !(0.25..=0.55).contains(&p.y) {
            continue;
        }
        let min_other = (0.80 - p.y) + 0.05;
        if (p.x - 0.0025).abs() < 1e-4 {
            checked_l += 1;
            let dl = geo.dist(v, 1) as f64;
            let dr = geo.dist(v, 2) as f64;
            assert!(dl < 0.15, "left inner thigh v={v} at {p:?}: dist to thigh.L too large ({dl:.4})");
            assert!(
                dr >= min_other && dr > 3.0 * dl,
                "left inner thigh v={v} at {p:?} bridged across 5 mm gap to thigh.R: d_L={dl:.4}, d_R={dr:.4} (want >= {min_other:.4})"
            );
        } else if (p.x + 0.0025).abs() < 1e-4 {
            checked_r += 1;
            let dl = geo.dist(v, 1) as f64;
            let dr = geo.dist(v, 2) as f64;
            assert!(dr < 0.15, "right inner thigh v={v} at {p:?}: dist to thigh.R too large ({dr:.4})");
            assert!(
                dl >= min_other && dl > 3.0 * dr,
                "right inner thigh v={v} at {p:?} bridged across 5 mm gap to thigh.L: d_R={dr:.4}, d_L={dl:.4} (want >= {min_other:.4})"
            );
        }
    }
    assert!(checked_l >= 20 && checked_r >= 20, "expected inner thigh verts, got {checked_l}, {checked_r}");

    // Geodesic binding must stay below the checker's bleed_weight threshold (0.05)
    // on both inner thighs (voxel bridging leaks ~0.48 onto the opposite thigh).
    let ctx = Ctx::new(&m, &CtxOpts { voxel_res: 64, clips: false, ..CtxOpts::default() });
    let w = fix::geodesic(&m, &ctx);
    for (v, p) in m.rest.iter().enumerate() {
        if !(0.25..=0.55).contains(&p.y) {
            continue;
        }
        if (p.x - 0.0025).abs() < 1e-4 {
            let bleed = weight_of(&w[v], 2);
            assert!(bleed < 0.05, "fix::geodesic leaked thigh.R weight ({bleed:.3}) onto left inner thigh v={v} at {p:?}");
        } else if (p.x + 0.0025).abs() < 1e-4 {
            let bleed = weight_of(&w[v], 1);
            assert!(bleed < 0.05, "fix::geodesic leaked thigh.L weight ({bleed:.3}) onto right inner thigh v={v} at {p:?}");
        }
    }
}

#[test]
fn narrow_gap_hanging_arm_does_not_bridge_to_torso() {
    // Single watertight torso + shoulder + hanging arm with a 5 mm air gap
    // between torso (+0.140) and inner arm (+0.145) for y in [0.55, 1.34].
    // Height is ~1.50 m -> at res = 128, h ~ 0.012 m (> 2x wider than 5 mm gap).
    let (pos, tris) = box_union_mesh(
        &[
            ([-0.14, 0.00, -0.07], [0.14, 1.50, 0.07]),
            ([0.14, 1.34, -0.05], [0.245, 1.48, 0.05]),
            ([0.145, 0.55, -0.05], [0.245, 1.34, 0.05]),
        ],
        0.025,
    );
    // Add child tip joint (`hand.L`) so glTF preserves `forearm.L`'s full length.
    let bones = vec![
        Bone { name: "spine", parent: None, head: v3(0.0, 0.15, 0.0), tail: v3(0.0, 1.40, 0.0) },
        Bone { name: "upper_arm.L", parent: Some(0), head: v3(0.195, 1.40, 0.0), tail: v3(0.195, 0.98, 0.0) },
        Bone { name: "forearm.L", parent: Some(1), head: v3(0.195, 0.98, 0.0), tail: v3(0.195, 0.60, 0.0) },
        Bone { name: "hand.L", parent: Some(2), head: v3(0.195, 0.60, 0.0), tail: v3(0.195, 0.56, 0.0) },
    ];
    let body_w: Vec<VW> = pos
        .iter()
        .map(|p| {
            let j = if p.x <= 0.141 {
                0
            } else if p.y >= 0.98 {
                1
            } else {
                2
            };
            vec![(j, 1.0)]
        })
        .collect();
    let f = Fixture { bones, body_pos: pos, body_tris: tris, cape_pos: Vec::new(), cape_tris: Vec::new(), body_w, cape_w: Vec::new() };
    let m = Model::from_glb(to_glb(&f)).unwrap();
    assert!((m.skel.tail[2] - v3(0.195, 0.60, 0.0)).len() < 1e-6, "forearm.L tail must reach y=0.60");

    let geo = geodesics(&m, 128, &[0, 1, 2]);
    assert_eq!(geo.solid.openings, 0, "armpit test mesh must be watertight");

    // Inner forearm vertices (x = +0.145, y in [0.65, 0.95]) sit 5 mm from torso,
    // but through the shoulder the path to spine is >= 0.55 m.
    let mut checked = 0usize;
    for (v, p) in m.rest.iter().enumerate() {
        if (p.x - 0.145).abs() < 1e-4 && (0.65..=0.95).contains(&p.y) {
            checked += 1;
            let d_fore = geo.dist(v, 2) as f64;
            let d_spine = geo.dist(v, 0) as f64;
            assert!(d_fore < 0.12, "inner forearm v={v} at {p:?}: dist to forearm.L too large ({d_fore:.4})");
            assert!(
                d_spine >= 0.50 && d_spine > 4.0 * d_fore,
                "inner forearm v={v} at {p:?} bridged across 5 mm gap to spine: d_fore={d_fore:.4}, d_spine={d_spine:.4}"
            );
            assert_eq!(geo.nearest(v).map(|e| e.0), Some(2), "inner forearm v={v} nearest bone must be forearm.L");
        }
    }
    assert!(checked >= 20, "expected inner forearm verts, got {checked}");

    let ctx = Ctx::new(&m, &CtxOpts { voxel_res: 128, clips: false, ..CtxOpts::default() });
    let w = fix::geodesic(&m, &ctx);
    for (v, p) in m.rest.iter().enumerate() {
        if (p.x - 0.145).abs() < 1e-4 && (0.65..=0.95).contains(&p.y) {
            let bleed = weight_of(&w[v], 0);
            assert!(bleed < 0.02, "fix::geodesic glued inner forearm v={v} at {p:?} to spine ({bleed:.3})");
        }
    }
}

#[test]
fn overlapping_shells_stay_connected_through_union() {
    // The standard mannequin has separate closed tubes for arms/legs that
    // penetrate slightly into the torso shell. Geodesics must cross overlapping
    // shell boundaries where the union stays interior.
    let m = Model::from_glb(to_glb(&fixtures::build("clean"))).unwrap();
    let joints: Vec<usize> = (0..m.njoints()).collect();
    let geo = geodesics(&m, 128, &joints);
    let spine3 = m.skel.joint_by_name("DEF-spine.003").unwrap() as u32;
    let upper_l = m.skel.joint_by_name("DEF-upper_arm.L").unwrap() as u32;

    // Every body vertex should reach at least one bone within cap.
    let body_verts: Vec<usize> = (0..m.nverts()).filter(|&v| !m.parts[m.vpart[v] as usize].piece).collect();
    for &v in &body_verts {
        let near = geo.nearest(v).expect("body vertex has nearest bone");
        assert!(near.1 < geo.cap, "body vertex {v} unreachable (cap={})", geo.cap);
    }
    // Upper arm vertices near the shoulder must still have a short geodesic path to spine.003.
    let near_shoulder: Vec<usize> = body_verts
        .iter()
        .copied()
        .filter(|&v| {
            let p = m.rest[v];
            p.x > 0.20 && p.x < 0.28 && p.y > 1.30 && p.y < 1.46
        })
        .collect();
    assert!(!near_shoulder.is_empty());
    for v in near_shoulder {
        let d_arm = geo.dist(v, upper_l) as f64;
        let d_chest = geo.dist(v, spine3) as f64;
        assert!(d_arm < 0.15, "arm vert {v} far from upper_arm.L ({d_arm:.3})");
        assert!(d_chest < 0.45, "arm vert {v} disconnected from torso through overlapping shell ({d_chest:.3})");
    }
}

#[test]
fn cut_cell_geodesics_handle_thin_parts_and_are_deterministic() {
    // A 1 m body with a thin 4 mm blade/fin protruding off the top (narrower than h = 1.0 / 64 ~ 15.6 mm).
    let (pos, tris) = box_union_mesh(&[([-0.10, 0.00, -0.06], [0.10, 0.70, 0.06]), ([-0.002, 0.70, -0.04], [0.002, 1.00, 0.04])], 0.025);
    let bones = vec![
        Bone { name: "base", parent: None, head: v3(0.0, 0.10, 0.0), tail: v3(0.0, 0.65, 0.0) },
        Bone { name: "fin", parent: Some(0), head: v3(0.0, 0.70, 0.0), tail: v3(0.0, 0.96, 0.0) },
        Bone { name: "fin_tip", parent: Some(1), head: v3(0.0, 0.96, 0.0), tail: v3(0.0, 0.99, 0.0) },
    ];
    let body_w: Vec<VW> = pos.iter().map(|p| vec![(if p.y < 0.70 { 0 } else { 1 }, 1.0)]).collect();
    let f = Fixture { bones, body_pos: pos, body_tris: tris, cape_pos: Vec::new(), cape_tris: Vec::new(), body_w, cape_w: Vec::new() };
    let m = Model::from_glb(to_glb(&f)).unwrap();
    assert!((m.skel.tail[1] - v3(0.0, 0.96, 0.0)).len() < 1e-6, "fin tail must reach y=0.96");

    let g1 = geodesics(&m, 64, &[0, 1]);
    let g2 = geodesics(&m, 64, &[0, 1]);
    assert_eq!(g1.near, g2.near, "geodesics must be bit-deterministic across runs");

    for (v, p) in m.rest.iter().enumerate() {
        let near = g1.nearest(v).expect("thin-part vertex must have nearest bone");
        assert!(near.1 < g1.cap, "vertex {v} at {p:?} became unreachable on thin part");
        if p.y > 0.76 {
            assert_eq!(near.0, 1, "thin fin vertex {v} at {p:?} should be nearest to fin bone");
        }
    }
}
