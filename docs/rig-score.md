# The rig checker's headline (2026-11-13)

What `weights check` now leads with, why, what was fixed in the checker
on the way, and the numbers the lines were set from. Supplementary
visual reports live outside this repository.

## Why

The 0-100 score is half the mean and half the single worst region, so it
counts faults rather than measuring quality: hand-made professional rigs
scored 3 to 70, the owner's own generated cast 20 to 78. The owner cannot
judge grey extreme poses and said what they do notice (2026-11-12): cloth
moving with the wrong bone, cloth not following its limb, stiff things
bending, limbs passing through each other, big stretch. The headline
measures those, on the game's own clips as well as on the poses.

## The headline

Per pose group (the game's clips, then the range-of-motion poses):

- **tears**: the share of the surface (area) inside failing findings.
  The vertex share is kept beside it (`tears_verts_pct`; the bench shelf
  and wrap-vs-graft reports used it).
- **findings as areas**, each named by part and side ("left thigh, outer
  side"), with the frame where it is worst and the amount, all on a body
  scaled to 1.75 m:
  - `wrong_bone`: a vertex is pulled more than 6 cm off where its own
    bones would put it by weight on a bone it does not belong to. It
    belongs to its own part's bones; the torso and head blend; torso skin
    may follow a limb's first bones (shoulder, upper arm, thigh: armpit,
    groin, a dog's flank) but not its forearm, hand, shin or foot; a limb
    never follows another limb.
  - `not_following`: skin round the middle of a limb (30-80% along the
    bone, within two limb radii) more than 5 cm and half a limb radius off
    the motion of its own bone and its children, held back by a bone off
    the limb.
  - `stiff_bends`: a part that should move as one piece is more than
    1.5 cm and 8% of its size off its best rigid motion. Stiff parts are
    pieces named for one (belt, buckle, pouch, plate, armour, bag,
    scabbard, bracer...) and small solid loose shells off the head.
  - `limb_through`: skin of one part inside another (arm into torso, leg
    into leg), per pair of parts at its worst frame. Reported beside the
    tears (`through_cm2`), not in them: the clips cause it as often as the
    weights (a hand swung into the chest, a body lying on its arm). A
    failing finding on the clips, a warning on the poses (full bends poke
    by nature).
  - `stretch` (past 3.5x, the owner's line of 2026-11-11), `volume`,
    `bleed`, `noise` (speckle that shows), `unweighted`, `piece_follow`,
    as areas. `piece_bones` (rfcheck's contract rule) stays in the
    findings list, not the headline.

The score, `pass` and the old findings list are unchanged in the JSON
(external pipeline callers read them; `fix` gates on them).

Since 2026-11-15 a claim can be judged blind at the game camera
(`weights blind`, `weights blind-score`; [`blind.md`](blind.md)): where a
judge and tears agree against the score, that claim's score verdict
is retired, and the `compare --zoom` headline ranks by pass then
tears on the same rule. Tears is the kept measure throughout.

## The clips

`--motion character.glb` plays the source's clips (default `walk,run,
attack_1,dodge,hit,death`) on the checked rig, put on by bone role:
per frame, each source bone's world rotation change from rest is applied
to the target bone of the same role after aligning their rest
directions, with a yaw when the rigs face different ways. Measured
against external retargeting output for the same rig without contact
easing: within 1.8 cm at every joint. Contact passes (which ease hands
out of the body) move wrists up to 12 cm more, so an already-animated
rig is best checked on its own embedded clips; role transfer handles rigs
with different skeletons or naming conventions (CesiumMan, Victoria, the
fox). Clips are sampled at 15 frames a second.

## What was wrong in the checker, and the fix

- **Only the first skin was read.** Every skin's joints now merge into
  one list; each primitive's rest shape comes from its own skin; writing
  maps back per skin and appends a joint a skin lacks.
- **Bone roles came from substrings** (`ring` in `Eye_Ring` made it a
  finger; `bone_N` skeletons had no roles). Roles now come from structure
  and the mesh (`core/src/anatomy.rs`): feet are chains ending at the
  floor, clustered by distance or by meeting low in the tree; legs climb
  from them to the joint holding no other foot; the heaviest legs win
  (skirt chains weigh less). Two legs and an upright spine is a humanoid:
  the spine runs from the hips (where the legs meet) to the joint owning
  the crown; arms are the subtrees reaching farthest to each side; the
  head is the joint above the chest with the most vertices. In a leg the
  thigh is the last joint at hip height and the knee the joint halfway
  down; in an arm the hand is the first joint past 72% of the arm's
  reach, the upper arm starts after a clavicle (a whole-word name hint
  inside the chain structure found) or at the first joint clear of the
  midline, and the elbow is the joint nearest the middle of the arc.
  Four legs: a quadruped (`poses/quadruped.ron`); other counts: legged
  (leg swings and bends). Sides come from position. On the owner's cast
  and graft the roles equal the old name roles exactly; on the shelf they
  fix CesiumMan and RiggedFigure (generic before), the fox, horse and pug
  (quadrupeds now), and the VRM arms (roll bones were taken for arms).
- **Bone tails by one global vote**: 30 grafted finger bones following
  Blender's +Y convention flipped it for the whole body and swung the
  thighs ~55 degrees (a false shin bleed). Each joint is now decided on
  its own: it ends at the child on its +Y axis, else at the child
  continuing the chain.
- **Rigid-parented parts were untested.** An unskinned mesh under a joint
  is loaded riding it (`Model::rigid`), checked (it can pass through the
  body), never written, never changed by `fix`.
- **Noise tripped on 24 of 25 shelf models.** A speckle now counts only
  where it shows (some pose kinks it off its neighbours by a third of an
  edge) and only when it stands out of its neighbours (a coarse mesh's
  joint vertex halfway between them does not). Low-poly rigs: 5-14
  findings each before, 0 now; Snow 65 before, 2.
- **Flat skeletons** (deform bones exported without the bones joining
  them: 309 of Victoria's 312 joints were roots) bent one bone at a time,
  so a forearm bend left the hand behind. Such joints are now linked by
  position (`Skeleton::link_flat`) and posed through the links.

## Calibration (`bench/rig_score.py`, 2026-11-13)

Tears on the hero's six clips, % of the surface:

- hand-made game rigs (Quaternius UBC male/female, knight, two robots;
  Khronos CesiumMan, RiggedFigure; both VRM samples; the MPFB base):
  median 0.36 (0 to 1.0; the knight 7.1, its armour collapses lying
  down in the death clip). The old score's median for them was 15.
- the owner's ten generated cast rigs: median 4.1 (1.0 to 10.6).
  Old score median 61.
- the game's hero file: 8.8.

The lines were set between the two groups: on these clips hand-made
rigs reach 4-8 cm (wrong bone) and 2-4 cm (not following), the cast
7-70 cm. Tried and left out, with numbers:

- the belt line (the ring of torso skin between the hips and the first
  spine joint) as a stiff part on the clips: hand-made waists bend as
  much in a run (3.3 cm) as the game hero's (2.5 cm);
- painted parts on a fused body (a band of one colour round the waist,
  a patch unlike its surroundings): over 36 rigs they found one patch,
  the tip of a beard; the cast hero's belt is the colour of the vest it
  sits on.

So a belt on a fused generated body is not found yet; a modelled-apart
or named one is.

### Creature calibration (`bench/creature_calib.py`)

`bench/creature_calib.py` calibrates tears on hand-made artist animals
against raw pre-`fix` generated creatures (before and after `weights fix`) on poses
and creature-gait clips (`--creature-motion PATH`, own clips, or `--gait`
built-in neutral locomotion clips), and pairs every generated creature's before/after
at the game camera (`weights blind`).

#### Raw pre-`fix` benchmark workflow

- Generated directories discover `<name>.glb`, `<name>/raw.glb`, or `<name>/unfixed.glb`
  (parallel to artist `<name>/truth.glb`), keeping benchmark evaluation pointed at
  un-fixed inputs rather than second-pass already-fixed rigs.
- Each measured entry inspects `<stem>.report.json` (`verts_changed`, `mean_l1_change`)
  and `<blind>.key.json` (`distinct_claims`, `max_claim_rmse`) to flag pre-fixed
  inputs whose before/after deltas are sub-pixel (`total_claims == 0`).
- Passing `--require-raw` fails Mode 1 (`--out`) or Mode 2 (`--score`) whenever any
  generated input looks already fixed (`verts_changed == 0` or zero distinct
  visible blind claims). Passing `--gait` enables synthesized `walk`, `trot`,
  `lunge`, and `attack` clips when no motion source or own animations exist.

The calibration `--score` gate passes only when artist medians stay below
generated medians, zero generated creatures worsen after `weights fix`, all
blind control and planted-swap gates pass, raw pre-`fix` checks pass (under
`--require-raw`), and the blind judge agrees with tears on at least 80% of
judged claims.

#### Real-corpus calibration runs (2026-11-20, `meshweights-1`, fleet `j92vghq2`)

Measured with `weights` on the cut-cell geodesic graph (post-`meshweights-8`
split, `fix_golden.sh --check` 105/105 on fleet `9avuccsq`) across two
real creature sets, each judged by a fresh blind visual judge before
opening answer keys. This is the last real run; nothing since has
re-measured it, so these numbers stand.

1. **Standard dev calibration (12 Quaternius/Khronos artist animals vs 5 bench creatures, poses only — bench refs were rigged `--no-clips`):**
   - Medians (poses tears %): **Artist `3.83%`** < **Generated before `fix` `4.24%`** → **after `fix` `2.62%`** (`0/5` worse; separation **PASS**). Artist clips median: `2.91%`.
   - Blind game-camera gates (40 pairs): **`5/5` controls PASS**, **`5/5` planted swaps caught by eye and tears (`planted_ok` + `planted_tears_ok`)**, **`0/30` non-control/non-planted claims visibly distinct** (`cant_tell` on all 30 — the 5 bench refs were already pre-fixed by the creature route, so a second `weights fix` pass produces sub-pixel differences at 75° / 3.0 m).
   - Overall `creature_calib.py --score` gate: **FAIL** (`total_claims == 0`, cannot establish `>= 80%` claim agreement on already-fixed refs; raw un-fixed creature rigs are needed to generate visible before/after claims).
2. **Held-out calibration (12 frozen non-Quaternius CC0/CC-BY held-out artists from `skintokens-1` vs 3 frozen held-out creatures from `meshweights-heldout`):**
   - Medians (**clips** tears %): **Artist `0.47%`** < **Generated before `fix` `2.59%`** → **after `fix` `1.96%`** (`0/3` worse; **5.5x clips separation PASS**).
   - Medians (**poses** tears %): **Artist `4.92%`** >= **Generated before `fix` `4.54%`** → **after `fix` `1.52%`** (`0/3` worse; **poses separation FAIL**).
   - Blind game-camera gates (23 pairs): **`3/3` controls PASS**, **`3/3` planted swaps flagged by tears**, **`2/3` planted swaps caught by eye** (`creature_02` missed on a symmetric `attack@0.53s` lunge under a thick mane, matching `docs/blind.md`), **`0/17` claims visibly distinct** (`cant_tell`).
   - Overall `creature_calib.py --score` gate: **FAIL** (poses separation failed, `creature_02` planted eye miss, `total_claims == 0`).
   - **Actionable checker finding:** Real-clip tears cleanly separate hand-made non-Quaternius animals (`0.47%` median; `kaiju`, `m2m_dragon`, `m2m_snake`, `m2m_spider`, `octopus_dense`, `whale` all `0.00%`) from generated creatures (`2.59%`), whereas generic 45° poses bend every joint of 55–107 bone artist rigs (`bee_dense` `30.08%`, `shark_dense` `12.44%`, `eagle` `7.39%`, `panda` `7.00%`) blindly and invert the median. Calibrating creature tears reliably requires creature-specific clips/ROM poses and raw (pre-`weights fix`) generated creature inputs.

#### Held-out sets are for scoring, not tuning

Two sets are frozen for scoring a finished change once: the 12 held-out
artist animals (from `skintokens-1`) and the 5 rigs in `bench/held_out.json`. Range-of-motion poses, planted
swaps and blind frame choice are tuned on the dev set only (the 12 dev
artist animals and the 5 bench creatures), then run once on
the held-out sets. A change whose design came from a held-out failure has
to be re-derived on the dev set before its held-out run counts.

2026-11-25: the 11-22..24 changes that were tuned on held-out failures
were undone: the softened creature range of motion (18° single bends,
26° chain spreads, unposed eye/horn/ear/claw/feather and small leaf
bones, softer leg and quadruped angles), the whole-limb-chain planted
swap, planted frames picked by pixel difference, and the synthesized
`asymmetric_stride` frame. None was measured on a real run. The default
range of motion is again ±45° bend and twist per weighted bone for
bodies without a preset, `poses/quadruped.ron` at its 11-20 angles, and
30°/25° leg swings with 40° lower-leg bends on other legged bodies. The
planted case is again one mirrored role pair shown at the frame where it
tears most. Tests pin each (`creature_clips.rs`
`deep_55_to_107_bone_creatures_keep_the_full_45_degree_generic_rom`,
`tests/blind.rs` `planted_swap_is_one_role_pair_at_its_worst_tearing_frame`).

#### Creature anatomy and gait clips

- **Structural creature anatomy (`anatomy.rs`):** Unpaired midline drooping chains (`DEF-tail.*`) are pruned before leg selection so heavy low tails never evict hind legs; quadruped anterior ribcage segments stay in `spine*`/`Torso` while `neck` and `head` separate cleanly even when neck mass exceeds skull mass; level-spine winged bipeds classify as `Legged` instead of `Humanoid`; structural roles cover `wing*.L/R`, `fin*.L/R`, `jaw`, and radial `limbK*` tentacles.
- **Neutral gait clips (`motion::creature_gait_clips`, `--gait`):** Synthesizes anatomy-driven `walk`, `trot`, `lunge`, and `attack` clips for clip-less non-humanoids when `--motion` is absent and `--gait` is passed. Opt-in; it adds a clip group and never replaces the range of motion.
- `poses/winged.ron` and `poses/multi_limb.ron` are softer opt-in presets (`--poses`), not the default.
- **Next empirical gate:** Re-run `bench/creature_calib.py --require-raw` end-to-end with raw un-fixed auto-rigged creature outputs (the frozen held-out creatures in `meshweights-heldout` are pre-fixed), tuned on the dev set and run once on the held-out sets. Until then the creature gate is **FAIL**.

## Known limits

- The Blender Studio rigs on the shelf are film rigs: their deform bones
  follow controls through constraints, IK stretch bones deform, and
  correctives are shape keys. Neither their deform-only export (flat)
  nor a game-skeleton rebuild in Blender poses like the rig does in
  Blender; five of ten read as humanoids after the rebuild. Their numbers
  measure those files as a game would play them, not the rigs.
- Four-legged and other bodies get no clips by default (the reference
  clips target humanoids); their default headline is on the poses. Pass
  `--gait` (built-in creature `walk`, `trot`, `lunge`, `attack`) or
  `--motion <clips.glb>` to add a clip group on un-animated creatures.
- Quaternius animals weight their hooves to IK foot bones under the root:
  bending a leg leaves the hoof behind. That is reported (wrong bone);
  their own clips would key those bones.
