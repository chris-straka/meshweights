# meshweights

Standalone, engine-agnostic skin-weight QA and repair for rigged game
characters and creatures, especially AI-generated ones (Tripo,
SkinTokens, UniRig). Built as an MIT-licensed Rust core + `weights` CLI
plus a GPL Blender extension (`blender/weightforge/`). GLB in, GLB out,
structural rig-contract clean (`rfcheck`-compatible).

Structural checkers verify static invariants (≤4 influences, normalized
weights). `meshweights` poses the rig through range-of-motion poses and
animation clips, measures what visibly breaks (tears, collapsed elbows,
a hand that follows the thigh, a cape on the wrong bone, speckled
weights), repairs only `JOINTS_0`/`WEIGHTS_0`, and proves the result with
numbers and before/after renders.

Status: humanoid QA, repair, and game-camera blind judging are in active
use alongside the Blender extension. Creature QA is not calibrated: its
gate (`bench/creature_calib.py --score`) is **FAIL** on the last real run
(2026-11-20, fleet `j92vghq2`) and has not been re-run
([`docs/rig-score.md`](docs/rig-score.md#creature-calibration-benchcreature_calibpy)).
Documentation index: [`docs/README.md`](docs/README.md).
Spec & roadmap: [`PLAN.md`](PLAN.md). Archived gates:
[`docs/archive/phase-0.md`](docs/archive/phase-0.md) … [`docs/archive/phase-5.md`](docs/archive/phase-5.md).

## Build

    cd rust && cargo build --release      # -> rust/target/release/weights

Pure Rust, permissive deps only (serde, serde_json, ron, png, rayon).

## Use

    weights check   hero.glb [--motion game_hero.glb] [--motion-clips walk,run]
                    [--gait] [--poses rom.ron] [--json] [--vertex-json f.json]
    weights show    hero.glb --out dir/ [--motion game_hero.glb] [--kind wrong_bone] [--top N]
                                                    # a close-up sheet per headline finding
    weights before-after raw.glb fixed.glb --out dir/ --motion game_hero.glb [--top N]
                                                    # the raw rig's worst areas on the clips, each
                                                    # beside the same place and frame on the other rig
    weights sheet   hero.glb --out sheet.png        # ROM pose sheet, bad verts red
    weights fix     hero.glb --out fixed.glb --sheet ab.png
                    [--method auto|smooth,geodesic,transfer,optimize]
                    [--source base.glb] [--candidate other.glb] [--skintokens]
                    [--all-regions] [--motion game_hero.glb] [--gait]
    weights compare a.glb b.glb --out ab.png        # same poses side by side
    weights compare a.glb b.glb --out ab.png --zoom --label-a THEIRS --label-b OURS --clips clips/
                                                    # which rig is better, by pass then tears (the
                                                    # score only where it agrees; where it points the
                                                    # other way the headline retires it): worst poses
                                                    # first, close-ups in clay, a loop for each pose
    weights probe   hero.glb --out cases/           # unmarked loops for a person to judge
    weights blind   a.glb b.glb --out pairs/ [--motion hero.glb] [--gait] [--seed N] [--top N] [--camera follow|talk] [--metrics metrics.ron]
                                                    # same-frame blind pairs per claim at the
                                                    # game camera, plus a can't-tell control and
                                                    # a planted bad case; key beside the folder
    weights blind-score pairs/ --answers a.json     # judge vs tears vs the old score;
                                                    # where judge and tears agree against the
                                                    # score, its verdict on that claim is retired
    weights dump    fixed.glb --out weights.json    # per-vertex weights for DCCs
    weights fixture all --out fixtures/             # test mannequins

Exit codes: 0 clean (for fix: the output passes), 1 faults found, 2
usage/IO error. `weights --help` has every flag.

**check** leads with what a person watching the character notices
(since 2026-11-13; method and calibration in
[`docs/rig-score.md`](docs/rig-score.md)), on the game's clips and on the
range-of-motion poses apart: **tears** (the share of the surface inside
failing findings) and each finding as an area named by part and side,
with the frame where it is worst: `wrong_bone` (a part moving with a bone
of another limb, or the torso with a forearm, hand, shin or foot),
`not_following` (mid-limb skin held back by a bone off the limb),
`stiff_bends` (a belt, buckle, pouch or plate bending), `limb_through`
(a limb inside another part: reported beside the tears, as the clips
cause it as often as the weights), `stretch` (past 3.5x), `volume`,
`bleed`, `noise` (only speckle that shows in a pose). Amounts are normalized
to a 1.75 m body scale. The clips are the file's own embedded animation
clips (if present), or `--motion character.glb`'s clips put on the rig by
bone role (measuring only; nothing is written). The 0-100 score
follows as the older measure; JSON keeps `score`, `pass` and the findings
below, and adds `headline` and `anatomy`.

**blind** proves a claim by eye: for every failing headline finding
of either rig, both rigs at the same worst frame from the game camera
(the game's follow camera, the whole body in frame, never a close-up
of the fault), in the character's paint, nothing marked and no rig
named — the judge answers left, right or can't tell, with a
can't-tell control (same rig both sides) and a planted bad case (a
mirrored limb's weights swapped) that must be picked out and flagged
by tears. **blind-score** lays the answers against tears (kept) and
the old score: where judge and tears agree against the score, that
claim's score verdict is retired. The `compare --zoom` headline
follows the same rule. Protocol and fixture numbers:
[`docs/blind.md`](docs/blind.md); `bench/blind_bench.py` drives runs.
`--camera talk` draws the pairs from the game's talk camera instead (the
close shot: 1.3 m from the face through a 38-degree lens, 24 degrees off
the front, head, shoulders and chest in frame, the frame following the face
wherever the pose carries it): claims whose skin is under that frame are
left out (the key lists them as `out_of_frame`), one more pair shows the two
rigs' faces at the first frame, and the planted case is shown at the frame
where the swap moves the in-frame skin most with the body upright.

Bone roles come from the skeleton's structure, not names (`core/src/anatomy.rs`):
feet are chains ending at the floor, legs climb from them, arms reach
farthest to each side of the spine, the head owns the crown; sides come
from position. Humanoids get `poses/humanoid.ron`, four-legged bodies
`poses/quadruped.ron`, other legged bodies leg swings and bends, anything
else per-bone bends (±45° bend and twist on every weighted bone, small
ones included). No clips by default unless supplied with `--motion`;
`--gait` adds built-in creature locomotion clips (`walk`, `trot`,
`lunge`, `attack`) as an extra group
([`docs/rig-score.md`](docs/rig-score.md#creature-calibration-benchcreature_calibpy)).
Every skin of the file is read; a mesh parented to a
bone without a skin is checked riding it (never written); each bone's
tail is decided on its own; a skeleton exported flat (deform bones
without the bones joining them) gets links by position. Findings: `D_STRETCH`, `D_VOLUME`,
`D_BLEED`, `D_NOISE`, `D_PIECE_FOLLOW`, `D_PIECE_BONES`, `D_UNWEIGHTED`
(fail), `D_INTERSECT`, `D_SEAM`, `D_JOINT_RANGE` (weights on a joint index
past the skin's joints: dropped on load), `D_OPEN_MESH` (the surface is
too open to tell inside from outside) (warn).
A pose that cannot run on the skeleton is a `pose not tested:` line (JSON
`skipped_poses`), never left out silently. Distances through the body are
measured in a voxel fill that small openings in the surface do not empty
(see `core/src/voxel.rs`); the report's `solid` says how many openings
there are and whether the fill can be trusted. Unknown flags are a usage
error (exit 2).

**fix** builds candidates: the input (always candidate 0), despeckle,
smooth, geodesic voxel binding, geodesic joint bands, transfer + inpaint,
optimize (LBS fit to dual-quaternion targets), and any `--candidate`.
Each candidate is measured on sets of failing regions (only their flagged
area changes, blended into the input at the seams), and an exact search
picks the plan that passes the gate if any can, then scores best
(`core/src/pick.rs`): a better candidate never picks a worse plan, and an
external candidate never leaves the fix worse than no candidate.
Refinement rounds re-measure against the current plan; every stage is
scored on the mesh and the best one that keeps the rules is written. No
region ends above 1.1x its input energy and no region gains a failing
finding, or the input comes back unchanged. Band widths (repair margin,
seam blend, smoothing, armpit/groin bands) are in limb radii, not edge
rings, so the fix works the same at any mesh density. Details and
evidence: [`docs/pick-and-bands.md`](docs/pick-and-bands.md).
`--skintokens` adds the ML candidate: SkinTokens weights for the input's
own skeleton (`skintokens skin` via `$SKINTOKENS_BIN` or `PATH`, ~20 s on
an Apple M4; opt-in, so default runs stay byte-deterministic).
Blender's automatic weights (bone heat) and external voxel weights on
the input's own skeleton can be passed as candidates the same way:
`bench/blender_weights.py IN OUT --method heat|voxel` creates them (heat
is bound at ten times the size, since it binds nothing on a small
figure), and `--candidate OUT` evaluates them. On test casts, heat and
voxel alone tear roughly twice as much, while winning regions on four of
ten rigs as candidates ([`docs/archive/review-2026-11-13.md`](docs/archive/review-2026-11-13.md), round 3).
On a body whose fused legs were separated in mesh space (an external unfuse
pass recording `extras.unfuse` in the GLB; see "Legs that share one shell"
below) the fix starts from each half following its own leg, and one more
candidate is built: `limbs-apart`.
It writes `fixed.glb` (in place: only JOINTS_0/WEIGHTS_0 bytes change,
never positions or bones), `fixed.report.json` (before/after per region,
method chosen, candidate scores), and with `--sheet` the A/B picture.

## Twist/helper bones

The standard humanoid skeleton uses twist helper bones at the upper arms
and thighs (`DEF-upper_arm_twist.L`, etc.). Each sits on its driver's
joint and turns by a share (0.5) of the driver's rotation; clips bake
that, and meshweights poses them the same way (`core/src/helpers.rs`:
node `extras.helper_bone`, else the `_twist` name). Helpers are not pose roles and not regions of their own, and
noise is judged on what a vertex does (a helper weight counts as half
driver, half parent), so unweighted helpers leave `check` unchanged.

`fix` weights them: `helper-band` widens each limb/body split around
the joint (a zone of 2.55 limb radii and smoothing 0.95 radii wide for
arms; 1.6 and 1.1 for legs, hips chain only so one thigh never spreads
into the other; on the 4.5k-vertex hero mesh that is the old 6 rings /
20 passes and 4 rings / 30 passes) and re-blends it onto the helper, and
`helper-band+optimize` refines that with the LBS-to-DQS fit. Each is
tried at 0.7x, 1x and 1.4x that width (`-narrow`, `-wide`): wider trades
stretch for collapse, and the right width depends on the body. Generic candidates leave helpers unweighted (re-blending
them too scored worse). A vertex on two limbs (the crotch, between the
thighs) re-blends both at once, sharing the body weight between them, so
it comes out the same on both sides whatever order the helpers are in
(until 2026-11-12 the first thigh's helper counted the other thigh as
body: hips 0.4, thighs 0.3 / 0.3 came out moving as 0.3 / 0.171). Why: SkinTokens' armpit/groin transition is one
or two edges wide, so a 90 deg swing tears those edges; widening it
alone collapses the joint, and the half-turn helper holds the volume.

A stylized humanoid game mesh, fresh SkinTokens rig (seed 0):

|                 | raw              | after `weights fix` |
| --------------- | ---------------- | ------------------- |
| without helpers | 47.3 (4 failing) | 62.8 (4)            |
| with helpers    | 47.6 (4)         | 68.8 (2)            |

Arms up and right arm forward are clean. Still failing: right thigh
stretch in `hip_forward.R` (9 verts, the crotch strip between the legs)
and left upper arm volume in `arm_forward.L` (7 verts, front fold).
Gate unchanged.

Later the same day (exact region pick, bands in limb radii; gate
unchanged): a 4,459-vertex humanoid mesh fixes 62.7 -> 76.0 and
**passes**, where it previously stopped at 75.7 with one left-arm
finding. The same character remeshed at 2.5k / 4.5k / 7k verts fixes to
77.2 / 76.2 / 76.3 (mean of 3 SkinTokens seeds; was 73.3 / 75.4 / 77.8),
passing in 5 of 9 runs (was 2). Under extreme overhead animations (`attack_3`)
shoulders raised past the head still require corrective blend shapes or clip limits.
See [`docs/pick-and-bands.md`](docs/pick-and-bands.md).

## Legs that share one shell

A generated human is one shell. Where it wears a tunic, a coat or wide
trousers, or stands with its thighs together, the shell below the hips is
one lump holding both thighs; the rig splits the lump down the middle, one
half on each thigh, and it tears there when a leg lifts (`hip_forward`).
No weights repair that: spread the split wider and the skin stops
tearing and collapses instead (measured, [`docs/archive/unfuse-log.md`](docs/archive/unfuse-log.md)). The
repair is a cut, and it is a mesh edit, so it is not made here: an
upstream unfuse step slits the lump along the plane between the legs
from the hip joints down, walls each half, and writes the cut into the
mesh's `extras.unfuse`. On such a file `fix` (`core/src/unfuse.rs`):

- starts from `own_legs`, not from the file's weights (a cut leaves them
  interpolated across it): skin whose outline (a horizontal cut through
  the vertex) lies wholly on one leg's side of the middle plane gives up
  its weight on the other leg's bones to their counterparts on its own
  (thigh to thigh, shin to shin, helper to helper);
- builds one more candidate, `limbs-apart`: that, and a falloff at the
  crotch: skin near the middle plane follows the hips at and above the
  cut's top and takes up its leg over 0.3 thigh lengths below it.

A file that was not cut gets neither, and `fix` runs on it as before
(`check` always reports a file as it is).
Bench and numbers: `bench/unfuse.py`, [`docs/archive/unfuse-log.md`](docs/archive/unfuse-log.md).
Probes (`rust/core/examples`): `fused_probe` (what joins two limbs,
cross-sections, failing vertices per pose), `worst_edges` (the most
stretched edges of a pose), `limbs_apart_try` (the weights step alone).

## Blender

`blender/weightforge/` (GPL-3.0-or-later): sidebar tab with Check (heat
map in the `weightforge_bad` color attribute), Pose Sheet, and Fix
(writes vertex groups, one undo). See [blender/README.md](blender/README.md).

## Checks

    cd rust && cargo fmt --all --check && cargo build --release && cargo test --release
    /Applications/Blender.app/Contents/MacOS/Blender --background --factory-startup \
        --python blender/tests/test_headless.py
    bench/run.py [models_dir]      # real assets (gitignored), A/B sheets + index
    bench/p3.py  [models_dir]      # P3 vs P2 on the 10 worst regions
    bench/unfuse.py run|table ...  # fused legs: fix and score a folder of rigs, lay runs side by side
    blender -b --python bench/rest_diff.py -- A.glb B.glb --out DIR    # rest pose unchanged by a mesh edit?
    blender -b --python bench/pose_paint.py -- A.glb B.glb --out DIR   # two rigs posed with their paint

## Layout

- `rust/core/` — scene (GLB IO, welding), poses, skinning (LBS/DQS),
  voxel geodesics, metrics, report, render, transfer/inpaint, fix,
  unfuse (legs apart), fixtures. MIT.
- `rust/cli/` — the `weights` binary + CLI contract test.
- `blender/` — extension + headless test.
- `poses/` — range-of-motion sets (RON).
- `bench/` — real-asset harnesses; `bench/models/`, `bench/results/` are
  gitignored.
- `tests/fixtures/golden/` — golden sheets.

## References

- Dionne & de Lasa, _Geodesic Voxel Binding for Production Character
  Meshes_, SCA 2013 (geodesic distances, binding falloff). Implemented from
  the paper; no reference code used.
- Abdrashitov et al., _Robust Skin Weights Transfer via Weight
  Inpainting_, SIGGRAPH Asia 2023 (match thresholds, harmonic inpainting).
  Implemented from the paper; no reference code used.
- Kavan et al., _Geometric Skinning with Approximate Dual Quaternion
  Blending_, 2008 (DQS targets for optimize).

License: MIT (Rust core, CLI); GPL-3.0-or-later (Blender extension).
