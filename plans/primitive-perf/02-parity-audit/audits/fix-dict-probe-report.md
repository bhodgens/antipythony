# Fix report — P09 dict probe unblocked + measured dict ratios (leaf ppfix-dictprobe)

Worktree: `/Users/caimlas/.git/worktrees/leaf-ppfix-dictprobe`
(branch `tier1-ppfix-dictprobe` @ 8a41c4f on main). Tests-only leaf: one
probe file + this report; no crates/ changes. CPython ref:
`/opt/homebrew/bin/python3.14` (3.14.7). Antipythony: fresh release
build of ap-cli at 8a41c4f (CARGO_TARGET_DIR=target-ppfix-dictprobe).

## What was wrong with the old probe

`p09_dict.py` built `KEYS_1M` (a 1M-element listcomp) at MODULE level
before any workload emitted anything, then ran 1M-element workloads at
6 passes each (warmup + best-of-5). Under the tier-0 tree-walker that
never reached the first `print` inside the harness's 120s probe
timeout → all P09 rows ERROR (`TIMEOUT after 120s`, slow-not-hung —
consistent with the sort-dict-json audit).

## Probe restructure (tests/bench/primitive/probes/p09_dict.py)

1. Emit-after-each-workload: every workload prints its JSON line
   immediately after its own timing; keys are built per-size right
   before that size's rows. Partial output survives any future timeout.
2. Fixed workload downgraded 1M → 32k entries, rows renamed
   `dict.build_32k` / `dict.get_32k` / `dict.getitem_32k` /
   `dict.iter_32k`. Rationale: dict behavior (hash growth, 2/3-load
   rehash regime, get/getitem/iteration) is fully exercised at 32k, and
   the tree-walker's SUPERLINEAR per-element cost makes anything larger
   infeasible inside 120s:
   measured single-pass dict build+lookup under antipythony:
   6k ≈ 0.17s, 16k ≈ 1.07s, 32k ≈ 4.0s, 49k ≈ 9.6s, 100k ≈ 35.9s,
   131k ≈ 63.9s. The original 1M fixed workload at 6 passes would be
   ~hours. NOTE: the leaf brief suggested 100k, but that measures
   ~18s/exec × 6 execs ≈ 215s > 120s timeout — 32k (2s/exec) is the
   largest size that fits the gate with margin.
3. Added `dict.getitem` and `dict.iter` workloads (kept `dict.build`,
   `dict.get`) so the per-op breakdown is visible in the harness rows
   themselves, not just hand probes.

Gate check (this host, quiet window): CPython whole probe 0.07s;
antipythony whole probe 76s < 100s. Both rc=0, 12 JSON lines each.

## Measured ratios vs CPython (32k-entry fixed workload; harness best-of-5)

| workload | CPython | antipythony | ratio |
|----------|---------|-------------|-------|
| dict.build (dictcomp insert) 32k | 0.586 ms | 1.821 s | 3,106x |
| dict.get (bound-method loop) 32k | 0.914 ms | 1.923 s | 2,104x |
| dict.getitem (`d[k]` loop) 32k | 0.895 ms | 1.814 s | 2,027x |
| dict.iter (insertion-order keys) 32k | 0.451 ms | 6.2 ms | 13.8x |

Same probe at the 48KiB-scaled size (6,144 entries):

| workload | CPython | antipythony | ratio |
|----------|---------|-------------|-------|
| dict.build | 0.111 ms | 69.9 ms | 632x |
| dict.get | 0.161 ms | 103.2 ms | 639x |
| dict.getitem | 0.133 ms | 72.1 ms | 543x |
| dict.iter | 0.084 ms | 1.20 ms | 14.2x |

All four workloads are int-keyed (identical work across interpreters,
per the probe convention). The hand audit's 135x number was STR keys;
int keys are far worse because of the Int interpreter-loop pathology.

## Per-op breakdown (the wave-2 targeting decision)

Dedicated probe at 32k int keys (`loop_only` = iterate the key list
counting; getitem/get = the probe lookup loops; all best-of-5):

| component | CPython | antipythony | ratio |
|-----------|---------|-------------|-------|
| interpreter loop alone (ns/element) | 12.6 | 57,100 | ~4,500x |
| `d[k]` getitem incremental (ns/op) | 26.5 | 359 | 13.5x |
| `d.get` incremental (ns/op) | 24.4 | 5,583 | 229x |
| get − getitem delta (bound-method dispatch, ns/call) | −2.1 | 5,224 | — |
| dictcomp build total (ns/element) | 18.8 | 57,123 | ~3,000x |

Attribution, in order of dominance:

1. **The interpreter loop over `Value::Int` is the whole story.**
   Looping 32k ints with NO dict work at all costs 1.83s under
   antipythony vs 0.40ms on CPython (~4,500x). Every workload's total
   (build 1.83s, getitem 1.84s, get 2.01s) sits ON that floor: the
   dictcomp insert adds ≈0 measurable cost over the bare loop, and
   `d[k]` adds only 359ns/op. This is the same ~35µs/element Int
   pathology documented in sort-dict-json.md §1 (Fix C) — confirmed
   here from the dict side. **Fix C (Int interpreter-loop pathology)
   is the single highest-leverage item for P09 — it is ~97% of every
   row.**
2. **`d.get` bound-method dispatch is real but secondary: 5.2µs per
   call on top of the loop** (get 5,583ns/op vs getitem 359ns/op). In
   CPython the delta is ~0 (get is marginally FASTER than getitem
   there). That 5.2µs is pure per-call overhead — BoundMethod mint +
   `methods::call_method` dispatch + cloned return value per hit
   (dict.rs items() clone shape per the audit). A dict-method fast
   path would cut the get workload from 2.01s to ≈1.84s (~15%); worth
   doing AFTER Fix C, not before.
3. **`d[k]` getitem dispatch: 359ns/op, 13.5x CPython — already
   cheap** relative to everything else. NOT the wave-2 target.
4. **The timing loop itself is NOT a confounder**: loop_only was timed
   with the same best-of-5 shape and is 30x cheaper than the lookup
   workloads on CPython; on antipythony it IS the workload.

**Wave-2 verdict: target the Int interpreter-loop pathology (Fix C),
not dict getitem or the dict structure.** The dict itself remains
ALREADY-PARITY (compact-dict transcription, CPython-exact probing and
resize — sort-dict-json.md §2); `dict.iter` at 13.8x (the only
non-Int-per-element-bound row) is consistent with a clone-per-iteration
cost, worth a look only after Fix C lands.

## Verification

- Probe green under both interpreters, rc=0: CPython 0.07s /
  antipythony 76s (both < 100s gate; harness timeout 120s).
- 12 rows land in the P09 bench slice — see
  `/tmp/ppfix_dict_p09.json` (scratch): all rows real PASS/FAIL
  verdicts vs CPython, zero ERROR. Every workload emitted its line.
- Scope respected: only `tests/bench/primitive/probes/p09_dict.py` +
  this report touched. run_bench.py, other probes, crates/ untouched.

## Honest-stop ledger

- One anomalous timing reading during breakdown measurement: a single
  breakdown run reported `build_s = 3.06ms` (vs 1.83s in four other
  isolated runs of the identical workload, min-of-5). Not
  reproducible; either a `perf_counter` glitch or a silently short
  build pass under the tree-walker. All reported numbers use the
  consistent readings; flagged because a one-off 600x timing error
  under our own interpreter could matter to future bench leaves.
- The 48KiB-scaled 131072-entry size from the original probe was
  dropped to 32768 (the 1MiB-scaled bucket) — at 131k the antipythony
  pass alone is ~64s and the 6-pass workload blows the 120s gate. The
  probe still spans 48KiB + 1MiB buckets via 6144/32768/32000.
- Int-pathology ROOT still not identified (unchanged from the audit's
  two spent attempts); this leaf adds only the dict-side confirmation.
