# meshweights-6 (helper): Kind-specific motion routing in bench/blind_bench.py

## What is needed
A `--ranking-claim` held-out batch contains both `"human"` and `"creature"` rigs (`bench/held_out.json`). Humans need external hero clips (`--motion hero.glb`) while creatures carry their own gait clips (`walk`, `trot`, `attack`, `idle`) and fail transfer if handed a humanoid clip source.

## What to build
Add `--human-motion PATH` and `--creature-motion PATH` to `bench/blind_bench.py`:
- For each run during pair generation:
  - If `held_out_kind == "human"` and `--human-motion` is set, pass `--motion <human-motion>` to `weights blind`.
  - If `held_out_kind == "creature"` and `--creature-motion` is set, pass `--motion <creature-motion>` to `weights blind`; if `--creature-motion` is not set and `--human-motion` IS set (and `--motion` is not set), omit `--motion` so `weights blind` plays the creature's own clips.
  - Otherwise fall back to `--motion` if given.
- Keep all existing flags and `gates/meshweights-2/test_blind_bench.py` passing.

## Allowed files
Only edit:
- `bench/blind_bench.py`
- `docs/blind.md` (optional)

## Gate
`bash gates/meshweights-6/gate.sh` must exit 0 with `GATE PASS`.
