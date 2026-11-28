# Follow-up: wasm probes that still diverge after leaves wasm-strplus + wasm-findings (2026-11-27)

Probes on the merged tree fdc86cb (post-merge, gates 42 suites ok / 30 passed / 0 failed).
Each DIFF reproduced identically on the findings leaf pre-merge, so none is a merge regression.
Each leaf's own corpus cases are byte-exact; these shapes sit outside them.

## W1 (UPGRADED): ALL i64-overflow math silently returns 0 (D2 + D3 both live)
- `2**62` -> correct. `2**63`, `2**64`, `2**62+2**62`, `4*(2**62)` -> all `0`.
  `12345678901234567890123` literal -> `4807115922877859019` (truncated to i64 wrap).
  `-(2**63)` -> `0`. Small-int neg works (`-5`).
- So: no bigint promotion exists anywhere on the overflow path (D2 as the auditor priced),
  AND the wrap produces 0 rather than the wrapped value for pow/add/mul edges (needs a
  look at binop smi arms - possibly the FN_WIDEN_128 call or i64.wrap behavior).
- This is D2 confirmed live at scale, beyond the auditor's diagnosis: it is not just
  `2**53` ordering; ANY overflow silently corrupts values with exit 0. Severity HIGH:
  silent data corruption, worse than a trap.
- Site: prelude.rs binop smi ADD/SUB/MUL arms + pow path + literal parsing (surrogates/
  lowering parse big literals to i64 wrap). Fix = D2's priced limb core.

## W2: `str in str` membership traps (D12a partial - list/tuple IN works)
- `print('a' in 'abc')` -> trap at `<wasm function 77>` (module body). CPython `True`.
- ISOLATED: list IN works (CF020 byte-exact via CLI: `True False/True True/...`),
  `not in` works, str ORDERING works (`'a' < 'b'` -> True).
- Root cause shape: `FN_ITER_CONTAINS`(65) scans $list/$tuple; a $str operand has
  no substring arm (CPython: `'a' in 'abc'` is a substring test, True).
- Site: prelude_gen.rs iter_contains body - add a $str/$str arm (byte-walk substring
  search; `'x' in ''` False, `'' in s` True per CPython).

## W3: missing-attr AttributeError raised but SILENTLY SWALLOWED (D12d partial)
- `a.missing` inside try/except AttributeError -> rc=0, EMPTY output (handler didn't run).
- Uncaught `a.missing` -> rc=0, empty - no traceback either. CPython: AttributeError + rc 1.
- The leaf's null-guard raises into a channel nothing catches and nothing reports. Silent
  swallow is the worst failure shape: data loss with exit 0.
- Site: lower.rs attribute-call path / the leaf's attr_get null-guard - verify it calls
  FN_EXC_RAISE (not a return-null); check the top-level EH wrap handles module-body raises.

## Method note
The leaves verified their fixes with corpus shapes; my post-merge probes diverge on adjacent
shapes. Both facts are true. The honest reading: fixes are partial for D3/D12; the addendum
rows must not be flipped to FIXED without these probes green.
