#!/usr/bin/env bash
# unsafe_report.sh — per-crate unsafe-Rust inventory + ratchet gate (issue #15).
#
# Pure bash + grep + git; no external tools, portable macOS/Linux.
#
# Usage:
#   scripts/unsafe_report.sh          print per-crate category table + totals
#                                     (incl. a 'tests-blocks' column counting
#                                     'unsafe {' lines under crates/*/tests/ —
#                                     issue #29: tests are separate crate roots,
#                                     NOT forbid-pinned, so they are surfaced
#                                     here without forbidding test unsafe)
#   scripts/unsafe_report.sh --diff   fail (exit 1) if ADDED lines in staged or
#                                     worktree-changed .rs files contain unsafe.
#                                     Bypass: ALLOW_UNSAFE=1 scripts/unsafe_report.sh --diff
#   scripts/unsafe_report.sh --clippy run cargo clippy across the workspace's
#                                     lib/bin targets and fail (exit 1) if the
#                                     undocumented_unsafe_blocks warning count
#                                     exceeds the baseline recorded in
#                                     docs/unsafe-inventory.md (issue #18 wave 3
#                                     ratchet; run informational first in CI).
#                                     BASELINE_CLIPPY_UNDOCUMENTED env or the
#                                     "baseline:" line in docs/unsafe-inventory.md
#                                     overrides; CLIPPY_PACKAGES overrides the
#                                     default -p list.
#                                     Bypass: ALLOW_UNSAFE=1 (exit 0, count printed)
#
# Committed baseline lives in docs/unsafe-baseline.txt. Regenerate with:
#   scripts/unsafe_report.sh > docs/unsafe-baseline.txt   (header line included by the script)
set -u

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || { echo "not a git repo" >&2; exit 2; }
cd "$ROOT" || exit 2

# ---- --clippy mode: undocumented_unsafe_blocks ratchet vs the recorded baseline ----
if [ "${1:-}" = "--clippy" ]; then
  PACKAGES="${CLIPPY_PACKAGES:-ap-cli xtask ap-ir ap-run ap-gc ap-obj ap-parse ap-ce ap-codegen-native ap-codegen-wasm ap-conform ap-host ap-pack}"
  # Baseline: env override wins, else parse the "baseline:" marker line from
  # docs/unsafe-inventory.md (wave 3 records: baseline: 0).
  BASELINE="${BASELINE_CLIPPY_UNDOCUMENTED:-}"
  if [ -z "$BASELINE" ]; then
    BASELINE="$(sed -n 's/^.*baseline:[[:space:]]*\([0-9][0-9]*\).*$/\1/p' docs/unsafe-inventory.md | head -1)"
  fi
  if [ -z "$BASELINE" ]; then
    echo "clippy ratchet: no baseline found (BASELINE_CLIPPY_UNDOCUMENTED or 'baseline: N' in docs/unsafe-inventory.md)" >&2
    exit 2
  fi
  if [ "${ALLOW_UNSAFE:-0}" = "1" ]; then
    echo "clippy ratchet: ALLOW_UNSAFE=1 — bypassing (baseline $BASELINE, gate skipped)"
    exit 0
  fi
  set -- $PACKAGES
  pargs=()
  for p in "$@"; do pargs+=(-p "$p"); done
  log="$(mktemp "${TMPDIR:-/tmp}/ug-clippy.XXXXXX")"
  # Some deny-lint pre-existing errors (e.g. ap-ce not_unsafe_ptr_arg_deref)
  # make clippy exit nonzero; we only ratchet the undocumented_unsafe_blocks
  # count, so capture warnings regardless of exit code.
  cargo clippy "${pargs[@]}" --no-deps >"$log" 2>&1
  clippy_rc=$?
  count="$(grep -c 'unsafe block without a safety comment' "$log" || true)"
  rm -f "$log"
  echo "clippy ratchet: undocumented_unsafe_blocks count = $count (baseline $BASELINE)"
  if [ "$count" -gt "$BASELINE" ]; then
    echo "clippy ratchet: GROWTH vs baseline — new undocumented unsafe block(s) (issue #18 wave 3)."
    echo "Add a '// SAFETY: <invariant>' comment above each new unsafe block, or regenerate the"
    echo "baseline in docs/unsafe-inventory.md consciously in the same commit."
    exit 1
  fi
  if [ "$clippy_rc" -ne 0 ]; then
    echo "clippy ratchet: note — clippy exited $clippy_rc on non-ratchet lints (count gate still green)"
  fi
  echo "clippy ratchet: pass (count <= baseline)"
  exit 0
fi

# Category greps (line counts within each crate's src/):
#   blocks   : 'unsafe {' blocks
#   fns      : 'unsafe fn' + 'extern "C"' declarations
#   transm   : transmute mentions
#   statmut  : static mut declarations
#   rawptr   : lines with a raw-pointer cast ( as *const / as *mut )
CNT_BLOCKS='unsafe[[:space:]]*\{'
CNT_FNS='unsafe[[:space:]]+fn\b|extern[[:space:]]+"C"'
CNT_TRANSM='\btransmute\b'
CNT_STATMUT='static[[:space:]]+mut\b'
CNT_RAWPTR='[[:space:]]as[[:space:]]+\*(const|mut)\b'

count_in() { # pattern dir -> count (0 when grep finds nothing / dir absent)
  if [ -d "$2" ]; then
    grep -rEc "$1" --include='*.rs' "$2" 2>/dev/null | awk -F: '{s+=$NF} END{print s+0}'
  else
    echo 0
  fi
}

if [ "${1:-}" = "--diff" ]; then
  # Files with changes: staged (cached) OR worktree. Fall back to comparing
  # against the merge-base with main when nothing is dirty (hook-friendly).
  files="$( { git diff --cached --name-only --diff-filter=ACMR 2>/dev/null
              git diff --name-only --diff-filter=ACMR 2>/dev/null
              git rev-parse --verify -q HEAD >/dev/null && \
              git diff HEAD --name-only --diff-filter=ACMR 2>/dev/null; } \
            | sort -u | grep -E '^crates/.*\.rs$' | grep -v '^crates/xtask/' || true )"
  if [ -z "$files" ]; then
    echo "unsafe ratchet: no changed .rs files to check"
    exit 0
  fi
  hits=""
  for f in $files; do
    [ -f "$f" ] || continue
    # added lines only (strip diff context/removals; skip +++ headers)
    added="$( { git diff HEAD -- "$f" 2>/dev/null; git diff --cached -- "$f" 2>/dev/null; } | sort -u )"
    # 'forbid(unsafe_code)' attributes are the PIN, not a new unsafe site.
    m="$(printf '%s\n' "$added" | grep -E '^\+[^+]' | grep -E '\bunsafe\b' | grep -v 'forbid(unsafe_code)' || true)"
    if [ -n "$m" ]; then
      hits="${hits}${f}: ${m}
"
    fi
  done
  if [ -n "$hits" ]; then
    if [ "${ALLOW_UNSAFE:-0}" = "1" ]; then
      echo "unsafe ratchet: NEW unsafe hits found but ALLOW_UNSAFE=1 set — bypassing:"
      printf '%s' "$hits"
      exit 0
    fi
    echo "unsafe ratchet: NEW 'unsafe' occurrences in changed files (issue #15 ratchet)."
    echo "Remove them, or bypass consciously with ALLOW_UNSAFE=1 plus a SAFETY: story in review."
    printf '%s' "$hits"
    exit 1
  fi
  echo "unsafe ratchet: no new unsafe occurrences in $(printf '%s\n' $files | wc -l | tr -d ' ') changed file(s)"
  exit 0
fi

# ---- report mode ----
printf '# unsafe inventory baseline — generated by scripts/unsafe_report.sh\n'
printf '# repo: %s @ %s\n' "$(basename "$ROOT")" "$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
printf '%-20s %8s %8s %8s %8s %8s %12s\n' crate blocks fns+externC transmute "static mut" rawptr-casts tests-blocks
tb=0; tf=0; tt=0; ts=0; tr=0; tv=0
for cdir in crates/*/; do
  c="$(basename "$cdir")"
  b="$(count_in "$CNT_BLOCKS" "$cdir/src")"
  f="$(count_in "$CNT_FNS" "$cdir/src")"
  t="$(count_in "$CNT_TRANSM" "$cdir/src")"
  s="$(count_in "$CNT_STATMUT" "$cdir/src")"
  r="$(count_in "$CNT_RAWPTR" "$cdir/src")"
  # Issue #29: tests/ integration targets are separate crate roots (not
  # forbid-pinned), so count their unsafe blocks in a dedicated column.
  v="$(count_in "$CNT_BLOCKS" "$cdir/tests")"
  printf '%-20s %8d %8d %8d %8d %8d %12d\n' "$c" "$b" "$f" "$t" "$s" "$r" "$v"
  tb=$((tb + b)); tf=$((tf + f)); tt=$((tt + t)); ts=$((ts + s)); tr=$((tr + r)); tv=$((tv + v))
done
printf '%-20s %8d %8d %8d %8d %8d %12d\n' TOTAL "$tb" "$tf" "$tt" "$ts" "$tr" "$tv"
