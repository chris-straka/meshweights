# meshweights-7: Cut-cell geodesic graph in place of the voxel graph

## What is broken today
`rust/core/src/voxel.rs` rasterizes triangles onto a coarse voxel grid (`cell = 1` surface, `cell = 2` interior), runs 26-neighbor Dijkstra across any adjacent solid voxels (`cell != 0`), and samples vertex distances via trilinear averaging over all 8 neighboring solid voxels.

When two body parts sit closer than ~1-2 voxel widths (e.g. a 5 mm gap between inner thighs or between a hanging arm and torso while `h ~ 12-14 mm` at `res = 128`):
1. Surface voxels on opposite sides of the air gap touch (or share a voxel), so Dijkstra bridges straight across the air gap instead of routing through the pelvis or shoulder.
2. Trilinear `sample()` averages across voxels inside the opposite limb.
3. Consequently, geodesic binding (`fix::geodesic`) and bleed detection confuse left/right thighs and arm/torso on close-limbed and generated characters.

## What to build
Replace the naive solid-voxel graph in `rust/core/src/voxel.rs` with a **cut-cell geodesic graph** (cf. `docs/research-2026-11.md`, paper 2608.11272):
1. **Surface-cut interior grid edges**:
   - Use exact ray-triangle intersections along grid rays (extending `ray_votes` / surface triangle tests) so grid edges between neighboring interior nodes are **cut** whenever the segment exits the solid into an air gap (even a sub-voxel gap on a single watertight shell, where ray parity drops from `inside > 0` to `0` and back to `> 0` inside the edge!).
   - Keep **overlapping shells** connected: where two closed shells overlap (`count` goes `1 -> 2 -> 1`, never dropping to `0`), edges stay open so multi-shell rigs (like the mannequin's arms/legs penetrating the torso) remain connected through the union.
   - Keep diagonal edges (or equivalent angular accuracy) where unobstructed so convex tube distances still meet `tests/holes.rs` (`<= 10% + 3*h` Euclidean accuracy) AND survive small mesh openings (`ray_votes >= 4` robustness).
   - Never allow a diagonal edge to cut corners across an air gap!
2. **Side-aware vertex attachment / sampling & thin parts**:
   - Do not average a surface vertex across voxels on the opposite side of an air gap: attach/sample each vertex only from interior grid nodes reachable without crossing out of the body (respecting surface triangles / inward normal), and support thin parts narrower than a voxel cell (`thickness < h`) so no vertex on a thin limb/fin/cape becomes unreachable (`cap`)!
   - Keep `bone_seeds` side-aware / interior-consistent as well.
3. **Preserve public API, performance, and determinism**:
   - Keep all public types/functions in `voxel.rs` (`Grid`, `Solid`, `MAX_CONTESTED`, `Geo`, `K`, `voxelize`, `voxelize_body`, `geodesics`) intact.
   - Bit-deterministic across runs and across macOS arm64 / Linux x86_64 (no `HashMap` iteration order in graph construction or sampling, use `total_cmp`).
   - Keep performance fast (comparable to existing `voxel.rs`, using spatial indexing / per-cell triangle buckets / ray intervals rather than brute-force all-triangle checks per edge).
   - All existing semantic tests (`holes.rs`, `never_worse.rs`, `p0_gate.rs`, `p2_gate.rs`, `density.rs`, `monotone.rs`, `creature_clips.rs`, etc.) plus `cut_cell_geodesic.rs` must pass with zero warnings and `cargo fmt --all --check` clean. (The pixel-exact PNG goldens in `p1_golden.rs` and `bench/fix_golden/` are re-verified and re-recorded after audit since replacing the geodesic graph is an intentional behavior change.)

## Allowed files
Only edit:
- `rust/core/src/voxel.rs` (or submodules under `rust/core/src/voxel/` if split cleanly while keeping `weightforge_core::voxel::*` exports identical)

Do not modify tests, `gates/`, or `bench/`.

## Gate
`bash gates/meshweights-7/gate.sh` must exit 0 with `GATE PASS`.
