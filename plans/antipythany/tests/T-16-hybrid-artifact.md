# T-16 — hybrid artifact (gates implementation leaf 04.05 hybrid artifact)

Authorship: written from the plan/contract documents ONLY —
`plans/antipythany/04-packaging-ecosystem/orchestrator.md` (leaf 04.05
Expose/Gate + the CLI verb surface), `plans/antipythany/03-wasm-backend/orchestrator.md`
(semantics-invariance rule, ap-host runner shape `ap-host run` from 03.02, the
tier-1 gap D4 note from 03.04), `docs/compat-contract.md` (C-24, C-27, C-29, C-30,
C-31, §9 D4), `plans/antipythany/master.md` (locked decision "Two artifacts: native
binary + separate .wasm"; hybrid is the third distribution shape, 04.05),
`plans/antipythany/tests/README.md`, and the exemplars
`tests/t09_binary/test_t09_binary.py` + `tests/t08_tier1/test_t08_tier1.py`
(cross-mode equality pattern). Per `tests/README.md` §Inputs, no implementation
source under `crates/` was read. Corpus programs under `tests/corpus/` are
read-only reference data (C-33).

## Purpose

Gate leaf 04.05. Expose (verbatim): "Native binary with the wasm module embedded
(payload section / include_bytes at link time); `app --emit-wasm` extracts;
embedded ap-host execution path behind a flag." Gate (verbatim): "one binary
passes native conformance slice AND its extracted wasm passes the wasm slice."

T-16 turns that into executable form around ONE idea: the **three-way byte
identity**. There are four execution legs of the *same* program — plain native
binary, hybrid binary run natively, standalone `build --wasm` module run under
ap-host, and the hybrid's EXTRACTED module run under ap-host — plus the pinned
CPython reference triple, and all five must be byte-identical on the only three
observable channels (stdout bytes, stderr bytes, exit code; C-24/C-27). The
rule that makes this a *conformance* claim rather than a smoke test is the Plan
03 semantics-invariance rule restated in C-31: "Native and wasm artifacts MUST
produce byte-identical stdout/stderr/exit code" (the plan tree's shorthand
"C-30 native-vs-wasm byte-identical rule" cites the contract's byte-identity
anchor; citation-alignment note G-1). D4 (wasm has no tier-1 in v1) is
timing-only and therefore never a permitted byte difference on these legs.

Program pair (task design): one case from `tests/corpus/values/`
(`V003-floordiv-mod-signs.py`, cites S-01-3/S-01-4/C-12) and one from
`tests/corpus/classes/` (`c001_class_basics.py`, cites S-03-01/S-03-04). Both
are inside T-09's pinned 30-case slice, so their expected triples are already
slice-blessed anchors; both are clean exits (rc 0, empty stderr) so C-30's
"exit code 0 reserved for clean exit" is a hard floor on every leg.

## Testability contract under test

- Surfaces invoked, all documented: `antipythany build <script> -o <app>`
  (02.06/Plan 04 verb table), `antipythany build --wasm <script>` (Plan 04 verb
  table, `-> ./app.wasm`), `antipythany build --hybrid` (Plan 04 verb table),
  `app --emit-wasm` (04.05 Expose), embedded-host execution flag (04.05
  Expose, spelling unpinned — G-5), `ap-host run <module>` (03.02 harness-flag
  text `--impl 'ap-host run'`; 04.04 also shows `./ap-host app.wasm`),
  `ap-conform run --corpus <case> --impl <cmd>` (00.04 CLI, as used by T-09).
- Artifact shape: per-program binaries (each corpus case compiles to its own
  artifact), so the suite drives build+run directly and expresses the
  ap-conform half as per-case `--impl` invocations (same reading as T-09 G-5).
- Extraction contract: `app --emit-wasm` run in an EMPTY working directory, rc
  0, exactly one `*.wasm` produced, `\x00asm` magic — the leaf text pins the
  flag, not the output location (G-5).
- Non-hybrid refusal: `--emit-wasm` on a plain native binary must fail with a
  CLEAN diagnostic — nonzero (non-signal) exit, non-empty stderr, no `.wasm`
  produced. Strictest reading of the leaf text (the flag is a hybrid
  capability); if implementers rule otherwise, amend the leaf text (G-4).
- Reproducibility of the embedded payload: extracted `.wasm` bytes == fresh
  `build --wasm` output bytes for the same source — the hybrid must not ship a
  silently different program than the portable distribution. Separately-marked
  xfail because wasm payload reproducibility is unpinned by the docs (G-3).
- Resolution order (mirrors T-09/T-08): `antipythany` = env `APX_CLI` →
  PATH → `target/{debug,release}/antipythany`; `ap-host` = env
  `APX_APHOST`/`APX_APHOST_CMD` → PATH → `target/{debug,release}/ap-host`;
  `ap-conform` = env `AP_CONFORM` → PATH → `target/{debug,release}/ap-conform`;
  reference = env `AP_CONFORM_REFERENCE_PYTHON`, default
  `/opt/homebrew/bin/python3.14`. Flag vocabulary overridable:
  `APX_HYBRID_BUILD_FLAGS` (default `--hybrid`), `APX_WASM_BUILD_FLAGS`
  (default `--wasm`), `APX_EMIT_WASM_ARGV` (default `--emit-wasm`),
  `APX_HYBRID_EXEC_FLAG` (default `--wasm`), `APX_T16_TIMEOUT`.
- C-23 honored: both pinned programs are finalizer-free deterministic scripts;
  nothing asserts GC/finalizer timing.

## Three-way byte-identity design (cites C-31 rule; plan shorthand C-30)

Structured as PAIRWISE equality tests exactly mirroring T-08's cross-mode
pattern, so a failure localizes to a leg, plus one aggregate lattice test as
the CI one-liner:

| Leg pair | Test | Rule |
| --- | --- | --- |
| pinned reference ↔ native | via each leg's comparison to `_pinned_triple` | C-01/C-02/C-27 |
| native ↔ hybrid (plain run) | `test_pair_native_vs_hybrid` | C-31 (AoT-vs-hybrid invisible), C-30 anchor |
| native ↔ standalone-wasm (ap-host) | `test_pair_native_vs_wasm_under_ap_host` | C-31 native↔wasm sentence; D4 timing-only |
| standalone-wasm ↔ extracted-wasm (ap-host) | `test_pair_wasm_build_vs_extracted_under_ap_host` | C-31 transitively; one program, three shapes |
| extracted-wasm ↔ pinned triple | `test_extracted_wasm_passes_wasm_slice` | 04.05 Gate half 2 verbatim (design-pending: byte-identity only, see below) |
| all five at once | `test_three_way_lattice_aggregate` | 04.05 Gate full |

The channels are never normalized or merged: three separate captures per run
(C-24), signal death mapped to the POSIX `128+signo` status a shell reports
(same convention as T-09).

## Spec-07 dependency declaration

`docs/spec/07-wasm-repr.md` (leaf 03.01, the WasmGC representation spec) does
NOT exist yet. Consequence, stated once and enforced by construction: every
wasm-side assertion in this leaf is **representation-independent** — it
compares the three conformance channels byte-for-byte, which the C-31/C-30
rule fully specifies with zero repr detail. This suite deliberately contains
NO struct-layout, string-encoding, bignum-strategy, or exception-shape
assertions; inventing them before spec 07 lands would gate on semantics the
docs have not fixed. When 03.01 lands, T-16 may GAIN representation-level
tests (tests only get stronger — C-33); none of today's tests weaken.

## CONTRACT-GAPS (findings to file back per README §Protocol 5)

- **G-1 — citation shorthand alignment.** The task/plan-tree shorthand calls
  the rule "C-30 native-vs-wasm byte-identical"; in
  `docs/compat-contract.md` that sentence lives in **C-31** ("Native and wasm
  artifacts MUST produce byte-identical stdout/stderr/exit code"), while C-30
  pins exit-code-0-is-clean-only. The suite cites both truthfully (C-31 for
  byte identity, C-30 for the rc-0 floor). If the plan tree intends a
  contract-internal cross-reference, correct the shorthand upstream — no clause
  content is being reinterpreted here.
- **G-2 — wasm-slice gate half is DESIGN-PENDING on spec 07.** 04.05's "its
  extracted wasm passes the wasm slice" is gated today at the byte-identity
  level only (see §Spec-07 dependency). No representation-level assertion is
  made. When `docs/spec/07-wasm-repr.md` lands, an amendment pass may add
  repr-aware checks; until then the byte lattice is the contract.
- **G-3 — embedded-payload reproducibility is unpinned.** No document read by
  this author states that the wasm payload embedded at link time is byte-
  reproducible against `build --wasm` output; 04.02's reproducibility gate text
  covers "build output" in the native-binary sense (linker build-id zeroing).
  `test_embedded_payload_byte_matches_wasm_build` is a SEPARATELY-marked xfail
  ("awaits repro decision" reason) so a failure there does not by itself block
  04.05's behavioral gate — the BEHAVIORAL identity legs (same triple under
  ap-host) stay in the 04.05 envelope regardless. Owner call: extend 04.02's
  reproducibility contract to wasm payloads, or document why the embedded copy
  may differ byte-wise while behaving identically.
- **G-4 — `--emit-wasm` on non-hybrid binaries: strictest reading.** The leaf
  text attaches the flag to the hybrid artifact; nothing states what a plain
  native binary does with it. T-16 asserts clean refusal (nonzero non-signal
  exit, stderr diagnostic, no `.wasm` produced). If implementers decide a plain
  binary accepts and no-ops the flag, that is a leaf-text amendment — the test
  must then be REPLACED by a strictly-equivalent documented contract, not
  silently weakened (C-33).
- **G-5 — flag spellings and emit output location are vocabulary gaps.**
  `--hybrid` / `--wasm` come from the Plan 04 verb table; the embedded-host
  execution flag's name and `--emit-wasm`'s output path/name are NOT pinned by
  any document. Defaults (`APX_HYBRID_EXEC_FLAG=--wasm`; empty-cwd discovery of
  exactly one `*.wasm`) encode the minimal reading; each is env-overridable so
  the orchestrator can align vocabulary without touching test bodies. Note the
  collision risk: `--wasm` is both a *build* flag (CLI) and our default
  *exec* flag (binary) — different binaries, so not contradictory, but the
  implementer may prefer a distinct exec flag name (e.g. `--run-wasm`).
- **G-6 — ap-conform's wasm-compile responsibility is unstated.**
  `ap-conform run --impl 'ap-host run'` (03.02 text) implies the harness
  compiles each corpus `.py` to wasm before invoking the impl; no read document
  confirms it. The direct `--corpus <case> --impl '<ap-host run>'` test is
  therefore xfail-marked under 04.05, and the byte-identity checks drive
  build+ap-host manually so the gate never depends on this ambiguity.
- **G-7 — reference-build drift (shared caveat).** Triples re-measured on
  `/opt/homebrew/bin/python3.14` = 3.14.7; C-01 pins 3.14.0 (provisional
  values; same open item as T-09 G-4 / T-06 GAP-7). The green-now half
  re-measures the reference on every pytest run so drift is loud and is a
  harness finding, never a 04.05 gate finding.

## Per-test citations (suite `tests/t16_hybrid/test_t16_hybrid.py`)

| Test | Assertion | Cites | Status today |
| --- | --- | --- | --- |
| `test_pinned_case_and_expected_triple_exist` ×2 | pinned program + complete expected triple present, slices = one values + one classes | C-33 (read-only corpus), 04.05 Gate slices | GREEN |
| `test_reference_reproduces_expected_bytes` ×2 | reference re-run == pinned bytes on 3 channels | C-01, C-02, C-27 | GREEN |
| `test_case_header_cites_clause_ids` ×2 | corpus header carries C-/S- ids | tests/README.md corpus rules | GREEN |
| `test_pinned_triple_is_clean_exit_anchor` ×2 | rc 0, stderr empty, newline-terminated stdout | C-30, C-25, C-27 | GREEN |
| `test_case_pair_is_one_values_one_classes` | pair composition exactly 1+1 | task design | GREEN |
| `test_hybrid_build_produces_executable` | `build --hybrid` → existing executable artifact | 04.05 Expose, master.md artifacts | XFAIL (04.05) |
| `test_hybrid_binary_native_green` ×2 | hybrid run natively == pinned triple | 04.05 Gate half 1; C-27/C-31/C-30 | XFAIL |
| `test_emit_wasm_extracts_module` ×2 | `app --emit-wasm` rc 0 → exactly one `\x00asm` file | 04.05 Expose; G-5 | XFAIL |
| `test_embedded_payload_byte_matches_wasm_build` ×2 | extracted `.wasm` bytes == fresh `build --wasm` bytes | payload reproducibility; G-3 | XFAIL (separate repro marker) |
| `test_pair_native_vs_hybrid` ×2 | leg N↔H byte-equal triples | C-31, C-30 | XFAIL |
| `test_pair_native_vs_wasm_under_ap_host` ×2 | leg N↔W byte-equal triples | C-31 native↔wasm sentence; D4 | XFAIL |
| `test_pair_wasm_build_vs_extracted_under_ap_host` ×2 | leg W↔H(wasm) byte-equal | C-31; master.md one-program-three-shapes | XFAIL |
| `test_extracted_wasm_passes_wasm_slice` ×2 | ap-host(extracted) == pinned triple | 04.05 Gate half 2; G-2 byte-only | XFAIL |
| `test_ap_conform_wasm_slice_via_ap_host_impl` ×2 | `ap-conform --impl 'ap-host run'` rc 0 | 03.02 CLI shape; G-6 | XFAIL |
| `test_three_way_lattice_aggregate` ×2 | all 5 legs pairwise-equal | 04.05 Gate full; C-24/C-27/C-30/C-31 | XFAIL |
| `test_emit_wasm_rejected_on_plain_native_binary` | clean refusal diagnostic, no artifact | 04.05 Expose strict reading; G-4 | XFAIL |
| `test_hybrid_binary_accepts_native_argv_passthrough` | argv accepted, triple unchanged | 04.05 native-first; C-31; cross-ref T-09 argv | XFAIL |
| `test_embedded_host_exec_flag_same_triple` ×2 | embedded-host flag path == native path triple | 04.05 Expose phrase 3; G-5; D4 | XFAIL |
| `test_hybrid_binary_runs_without_python_on_path` | PATH-isolated run byte-clean | master.md static-binary UX; T-09 G-7 precedent | XFAIL |

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t16_hybrid -q
```

From the repo root. Today: **9 passed, 24 xfailed, 0 errors** (33 total.
Green 9 = 2 manifest + 2 reference-reproduce + 2 header-cites + 2 clean-anchor
+ 1 pair-composition. XFAIL 24 = 1 hybrid-build seam + 2 native-green +
2 emit-extract + 2 payload-repro (separate marker, G-3) + 2 native↔hybrid +
2 native↔wasm + 2 wasm↔extracted + 2 wasm-slice-direct + 2 ap-conform-via-
ap-host + 2 lattice + 1 plain-binary refusal + 1 argv passthrough + 2
embedded-host flag + 1 PATH isolation).

## xfail policy

- Everything antipythany-side is `xfail(strict=False)`: 04.05 is unimplemented
  (no `--hybrid` build, no `--emit-wasm`, no ap-host binary), so bodies fail
  inside the envelope (missing binary / failed build / missing artifact — never
  a vacuous pass: each seam asserts artifact existence, same rule as T-09 G-6).
- After 04.05 lands: XPASS is expected and reported; any FAIL is a gate failure
  — fix the implementation, never the test (master.md protocol; C-33). The
  payload-reproducibility test keeps its own marker until G-3 is resolved
  (decision or contract extension); do not merge it into the 04.05 envelope.
- The green-now half must STAY green; a break means the reference drifted
  (C-01/G-7) — harness finding, not a 04.05 finding.

## Protocol notes

- Tests commit BEFORE 04.05 completes (README §Protocol 3); the gate runner is
  an agent that wrote neither these tests nor the code (§Protocol 4).
- No file under `tests/corpus/` was created or modified; the two pinned
  programs and their expected triples are read-only references. New files are
  confined to `tests/t16_hybrid/` and this document.
- Findings G-1..G-7 are filed back with this document per README §Protocol 5.
