# NEXT — fp-bench-parity leaf (03-rollout/02-bench-parity)

Branch `fp-bench-parity` @ `e00fcb4`, worktree
`/Users/caimlas/.git/worktrees/leaf-fp-bench`. Owned files only: the 64-row
JSON, both charts, `docs/fast-path-report.md`, the README perf section,
`docs/benefits-vs-cpython.md` sync, and this note. No Rust source, gate
scripts, or HANDOFF doc touched.

## Outcome — exit gate RED, reported honestly

Fresh release binary: `target-fp-bench/release/antipythony`, SHA-256
`068cc7f20c70d167c268be17c787e6bf35cc986661d8406a45746bc186914160`
(mtime 17:34 MST, no newer sources; one target dir, `CARGO_BUILD_JOBS=4`).
Full 64-row bench in **candidate bytecode mode** (`APX_TIER=bytecode` SET
for the bench leg; `env -u APX_TIER_STATS`): **64 rows, 28 PASS / 36 FAIL /
0 ERROR**. Charts regenerated from the new JSON with the committed
generators (`gen_chart.py`, `gen_speedup_chart.py`).

Master exit gate (`plans/fast-path/master.md`):

- parity clause: **FAIL** — all 18 walk-bound rows below 1.0x:
  `dict.getitem` 0.205/0.267/0.268x, `dict.build` 0.181/0.192/0.192x,
  `list.sort` 0.168/0.112/0.146x, `sorted()` 0.828/0.849/0.811x,
  `str.join` 0.193/0.230x, `str.split` 0.451/0.509x, `json.loads` 0.509/0.289x.
- simple mean vs CPython **3.292x** (floor 1.5x) — PASS. Geomean 1.180x
  (reported, not a gate). Mean-vs-target 2.123x; geomean-vs-target 0.949x.

No gate was weakened and the JSON was not doctored. Per the leaf contract
("if a gate fails, STOP and report the exact failure"), this note records
the failure as the primary result; the deliverables (report/charts/docs)
are committed with the honest numbers, and the parent should treat the
parity clause as unmet.

## Why parity is unmet (measured, not assumed)

The primitive probe hot loops are MODULE-LEVEL; tier dispatch enters via
`call_function`, so module-level frames stay on the tree-walker even with
`APX_TIER=bytecode`. Proof: `[BC-STATS] compiles=6 fallbacks=3 execs=79`
on P09 (function-shaped helpers run bytecode) while `[TIER-STATS] execs=0`.
This is the same caveat the superinstructions leaf documented (§4: primitive
rows are NOISE for bytecode fusions). Bytecode did lift function-shaped
dict helpers (~0.01x -> 0.24-0.28x for `d.get`, ~0.06x -> 0.20-0.27x for
getitem) and `sorted()` (~0.4x -> ~0.8x), but `list.sort` (0.11-0.17x),
join/split and json.loads remain tier-0-floor-bound. Closing those needs
function-shaped probe loops or a module-level bytecode entry (separate
leaf), plus the already-filed homogeneous compare/bulk-join/nursery
levers — NOT a re-interpretation of this gate.

## Standard battery (final tree, clean env)

All env-sanitized (`env -u APX_TIER -u APX_TIER_STATS -u APX_TIER_SEAM_OVERRIDE
-u APX_BP_STATS -u APX_OSR_STATS -u APX_CALLDBG`):

- `fix_bytecode.py --other <fresh bin>`: **13/13**, 0 mismatches.
- `diff_b64` 2132/0; `diff_hex` 1976/0; `fix_int` 18653 lines byte-identical;
  `fix_json` 127/0; `fix_sort` 42/0; `fix_random` pass.
- `fix_sha_zlib` 818 lines: tree0 vs bytecode **0 diff**; vs CPython 62 diff
  lines (31 CORRUPT rows), all the documented zlib-rs message-granularity
  residual, zero non-CORRUPT divergences. (CLI rejects an extra positional
  datafile arg — the driver's default path `pfz_data.bin` beside the script
  was populated from a sibling driver-generated copy, sha256
  `55ef78a742d4379372b9f905963c1956372c724fe97a4c6835cd3ec10cd01e20`.)
- `ap-conform` values **50/50 + 50/50**, stdlib **48/48 + 48/48** in tree0
  AND bytecode.
- `tier_seam_differential` **3/3**, `ic_invalidation` **9/9**,
  `cargo test -p ap-bytecode -p ap-run --release` **22 result blocks /
  282 passed / 0 failed / 1 ignored** (baseline 274 + 3 seam + 5
  fused-opcode tests; 1 pre-existing ignore).
- Byte-exactness held everywhere; only performance parity failed.

## Files / reproduce

See `docs/fast-path-report.md` for the architecture, full 64-row
tree0-vs-bytecode-vs-CPython table, gap table and reproduce commands. Logs
were captured worktree-locally (`.fp-*.log`) during the run and removed
before commit; results are transcribed in the report and this note. No default tier was
flipped; the USER GATE in `03-rollout/01` stays pending.

## Next leaves (filed, not done here)

1. Module-level bytecode entry (or function-shaped probe loops) so the
   walk-bound rows actually measure bytecode dispatch — prerequisite for a
   meaningful parity re-gate.
2. Homogeneous int-list compare arm (`list.sort`), bulk list-of-str join,
   str.split per-piece allocation, json.loads nursery/batching.
3. Re-run the exit gate after (1)+(2); only then reconsider the default flip.
