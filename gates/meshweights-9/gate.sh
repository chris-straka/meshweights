#!/usr/bin/env bash
# Gate for meshweights-9 (game-tools docs/plans/meshweights.md): the docs say what is true.
#  - scope: only PLAN.md, README.md and docs/ changed by the worker (so the code and every
#    test are as they were: `cargo test --release` can't change);
#  - `grep -rniE "inbox|dailies|gen character" PLAN.md README.md docs` prints nothing;
#  - PLAN.md's title names meshweights, not weightforge;
#  - no doc lost more than a fifth of its lines (the history stays; wording changes only);
#  - every file README.md and PLAN.md link to by relative path still exists.
# Builds nothing, starts nothing. Prints PASS/FAIL per check, ends with GATE PASS or GATE FAIL.
set -u
cd "$(git rev-parse --show-toplevel)" || exit 2
SLUG=meshweights-9
FAILS=0
pass() { echo "PASS  $1"; }
fail() { echo "FAIL  $1"; FAILS=$((FAILS + 1)); }
show() { sed 's/^/      /' | head -n 30; }
BASE=$(git log -1 --format=%H -- gates/$SLUG)
[ -n "$BASE" ] || { fail "cannot find the commit that added the gate"; BASE=HEAD; }

echo "== scope"
mine=$(for c in $(git log --format=%H --grep "^carbon $SLUG: " "$BASE..HEAD"); do git diff-tree --no-commit-id --name-only -r "$c"; done)
other=$( (echo "$mine"; git status --porcelain | cut -c4-) | grep -v -E '^((PLAN\.md|README\.md)|docs/.*)?$' | sort -u)
if [ -z "$other" ]; then pass "only PLAN.md, README.md and docs/ changed"; else fail "other files changed:"; echo "$other" | show; fi

echo "== stale words"
hits=$(grep -rniE "inbox|dailies|gen character" PLAN.md README.md docs)
if [ -z "$hits" ]; then pass "no inbox, Dailies or 'gen character' in PLAN.md, README.md, docs"; else fail "stale words left ($(echo "$hits" | wc -l | tr -d ' ')):"; echo "$hits" | show; fi

echo "== PLAN.md title"
t=$(grep -m1 -E '^# ' PLAN.md)
if echo "$t" | grep -qi "meshweights" && ! echo "$t" | grep -qi "weightforge"; then pass "PLAN.md's title: $t"; else fail "PLAN.md's title: $t"; fi

echo "== history kept"
lost=0
for f in PLAN.md README.md $(git ls-tree -r --name-only "$BASE" docs); do
  nb=$(git show "$BASE:$f" 2>/dev/null | wc -l | tr -d ' ')
  if [ ! -f "$f" ]; then fail "$f was removed"; lost=1; continue; fi
  n=$(wc -l < "$f" | tr -d ' ')
  if [ "$n" -lt $((nb * 4 / 5)) ]; then fail "$f: $n lines, base $nb (more than a fifth gone)"; lost=1; fi
done
[ "$lost" -eq 0 ] && pass "every doc keeps at least four fifths of its lines"

echo "== links"
miss=""
for f in PLAN.md README.md; do
  for l in $(grep -oE '\]\([^)#:]+' "$f" | sed 's/^](//'); do
    [ -e "$(dirname "$f")/$l" ] || miss="$miss $f:$l"
  done
done
if [ -z "$miss" ]; then pass "every relative link in PLAN.md and README.md resolves"; else fail "broken links:$miss"; fi

if [ "$FAILS" -eq 0 ]; then echo "GATE PASS"; exit 0; else echo "GATE FAIL ($FAILS)"; exit 1; fi
