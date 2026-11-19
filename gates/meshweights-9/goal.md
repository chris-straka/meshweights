# meshweights-9: the docs say what is true

Repo: meshweights (`~/Games/_blender/meshweights`). One goal, this repo only. Docs only: no
code, no tests.

## Why

The owner retired the inbox and Dailies (review now happens in chat, with pictures). The
plan and the phase notes still send results "to the inbox", ask for "owner inbox sign-off",
and describe genforge's `gen character` chain as the place `weights` runs. The plan's title
still says weightforge, the tool's old name.

What is true today (2026-11-19):

- The tool is **meshweights** (folder and repo); the command stays `weights`; the Blender
  add-on folder is still `blender/weightforge` (don't rename it).
- The weight gate runs in the two rig routes: motion's `tools/tripo_rig.py` (people: `weights
  fix`, then `weights check`) and skintokens' `tools/creature_route.py` (creatures). genforge
  is now called assetgen (command `gen`); its character chain still calls the weights adapter,
  but it is not where the routes run.
- Results are judged in blind picture pairs at the game camera (`docs/blind.md`), and the
  owner sees pictures in the chat when a call is theirs. There is no inbox and no Dailies.

## What to do

1. `PLAN.md`: title `# meshweights — build plan`. Reword every line that names the inbox,
   Dailies or `gen character` so it says what is true (above): for example "for the inbox"
   becomes "for a blind pair or the chat"; the P4 and P5 rows keep their goal and numbers but
   no longer route results to the inbox or name `gen character`.
2. `docs/phase-4.md`, `docs/phase-5.md`: these are history. Keep every number, date and
   result; reword only the inbox, Dailies and `gen character` phrases (for example "the
   owner's sign-off in the chat"; "assetgen's character chain"). Don't delete paragraphs.
3. `README.md` and the other `docs/*.md`: the same, wherever the grep below finds a hit.
4. Nothing else changes. Old names inside history (genforge, weightforge) may stay where
   they describe the past; don't rewrite the history beyond the banned phrases.

## Evidence

`bash gates/meshweights-9/gate.sh` prints `GATE PASS`. It checks that only `PLAN.md`,
`README.md` and `docs/` changed, that `grep -rniE "inbox|dailies|gen character" PLAN.md
README.md docs` prints nothing, PLAN.md's title, that no doc lost more than a fifth of its
lines, and that the relative links in PLAN.md and README.md resolve. Because nothing outside
the docs may change, `cargo test --release` can't change; you don't need to run it.

## Out of scope; don't

- Don't edit `gates/`, `rust/`, `bench/`, `tests/`, `blender/`, `poses/` or `AGENTS.md`.
- Don't change any number, threshold, date or result in the docs.
- Don't name any other game, company or franchise.
- Don't start Blender or build anything on this Mac.
