# Test-tree tracking (orchestrator-maintained)

Updated 2026-10-16. Test leaves are authored from the spec pack only
(docs/spec/01..06 + docs/compat-contract.md + master.md), by agents that never
read implementation source. "Green/xfail" counts are from the authoring-agent's
final run, re-verified by the orchestrator full-tree run: 1164 passed,
2 skipped, 784 xfailed, 25 xpassed, zero collection errors.

| Test leaf | Gates | State | Suite path | Notes |
| --- | --- | --- | --- | --- |
| T-01 parse diagnostics | 01.01 | AUTHORED (committed d3a5277) | tests/t01_parse_diagnostics | 105 tests; seam gap G-1: ap-parse-check bin needed to run gate |
| T-02 IR shape | 01.02 | AUTHORED (committed 81126cc) | tests/t02_ir_shape | 21 tests; xpass since 01.03 ap-ir-dump landed |
| T-03 lowering semantics | 01.03 | AUTHORED (committed 424690e) | tests/t03_lowering_semantics | 20 corpus + scope goldens |
| T-04 IR verify | 01.04 | AUTHORED (committed 424690e) | tests/t04_ir_verify | CPython-pinned probes; ap_verify_dump bin in tree |
| T-05 values | 02.01 | AUTHORED (committed d3a5277) | tests/corpus/values | 47 cases, 141 expected-byte files |
| T-06 classes | 02.03 | AUTHORED (committed d3a5277) | tests/corpus/classes + exceptions | 48 cases; found spec bugs -> A-3, A-4 |
| T-07 gc/finalizer | 02.04 | AUTHORED (committed 2057bf8) | tests/corpus/gc | 17 cases, D1-timing discipline |
| T-08 tier-1 transparency | 02.05 | AUTHORED (absorbed d75c0a4) | tests/t08_tier1 | cross-mode byte-identity + bench harness stub |
| T-09 binary conformance | 02.06 | AUTHORED (committed 2057bf8) | tests/t09_binary | 30-case slice + exit-code cases |
| T-10 threading | 02.07 | AUTHORED (absorbed d75c0a4) | tests/corpus/threading | 18 cases, 3-run deterministic pins |
| T-11 stdlib-v1 | 02.08 | AUTHORED (committed 2057bf8) | tests/corpus/stdlib | 48 cases + scope table; ~30 spec-05 wording gaps filed |
| T-12 wasm parity | 03.02-03.04 | BLOCKED | - | needs docs/spec/07-wasm-repr.md (lands with leaf 03.01) — spec 07 authored (S-07-*), unblocking T-12/T-13 authoring per leaf 03.01 exit gate |
| T-13 host/wasi | 03.05 | BLOCKED | - | same dependency |
| T-14 cli/cache | 04.01-04.02 | AUTHORED (committed 71193a6) | tests/t14_cli | 22 tests; uv 0.11.29 present |
| T-15 release/install | 04.03 | AUTHORED (committed 71193a6) | tests/t15_release | 19 tests |
| T-16 hybrid artifact | 04.05 | AUTHORED (committed 71193a6) | tests/t16_hybrid | 33 tests; C-30/C-31 shorthand flagged G-1 |

## Open orchestrator decisions (queue)

1. Spec-05 revision pass: T-11 filed name-list errors (fnmatch.filterfalse,
   io.Buffer, operator count, codecs BOM names, collections.abc count) that
   do not exist on the reference. Needs one amendment wave against spec 05.
2. random row: C-05 implies IN, S-05-19 says OUT. Blocks the 02.08 gate
   declaration. Needs a ruling (PR against spec 05 or compat contract).
3. Seam bins: ap-parse-check (T-01 G-1) must be added to leaf 01.01 Expose
   or an equivalent surface, else the gate stays unrunnable.
4. Reference drift: all expected bytes recorded on 3.14.7; C-01 pins 3.14.0.
   Re-verification owed when a 3.14.0 build is available.
5. OUT-of-corpus vs registry-OUT wording split in spec 05 (~15 rows, T-11
   GAP-28). Decide vocabulary once, amend all rows.
