# T-09 — binary conformance (gates implementation leaf 02.06 AoT static binary)

Authorship: written from the spec pack + contract ONLY (`docs/compat-contract.md`
C-01..C-03, C-23..C-31, `docs/spec/06-observable-behavior.md` §6.4/6.5 anchors,
`plans/antipythany/master.md`, `plans/antipythany/02-native-backend/orchestrator.md`
02.06 Expose/Gate, `plans/antipythany/00-foundation-contract/orchestrator.md` 00.04
ap-conform CLI text, `plans/antipythany/04-packaging-ecosystem/orchestrator.md`
verbs + 04.02 gate), per `plans/antipythany/tests/README.md`. No implementation
source under `crates/` was read.

## Purpose

Gate leaf 02.06: the **AoT static binary** is the artifact under test — not the
JIT, not the CLI's `run` verb. The 02.06 Gate text is: "`./app` runs a hello-world
+ a classes corpus program; binary <= 15MB musl; conformance suite runs against the
BINARY, not the JIT, for a 30-case slice." T-09 turns that into executable form:

1. **Build seam**: `antipythany build <file> -o <app>` produces an existing,
   executable file (leaf 02.06 Expose; the CLI surface documented in Plan 04).
2. **Standalone run**: the built binary runs the hello-world program and a classes
   corpus program with byte-identical stdout/stderr/rc vs the reference (C-27),
   and runs under **PATH isolation** — a stripped `PATH=/usr/bin:/bin` with no
   `python3.14` reachable — because a static binary must be self-sufficient (the
   whole point of AoT; C-31's "AoT-vs-JIT differences MUST NOT change observable
   output" is the clause that makes this a *conformance* run, not a smoke test).
3. **30-case ap-conform slice against the BINARY**: for each pinned corpus case,
   build that case into its own binary and run
   `ap-conform run --corpus <case> --impl <built-app>` (00.04 CLI contract).
   The binary IS the impl command; no `--tier` flag applies (AoT has no tiers).
4. **Exit-code contract in binary form** (C-28/C-29/C-30, S-06-11/S-06-12): 12
   inline cases under `tests/t09_binary/cases/` covering `sys.exit(int)` incl.
   negative, `sys.exit(str)`, bare/None exit, non-int object with `__str__`,
   `[1]` list payload, uncaught exception (traceback shape + rc 1), thread
   `SystemExit` not setting process rc, and top-level `KeyboardInterrupt` (rc 130).
5. **Artifact properties**: binary size report + the musl 15MB bound as a
   mode-gated assert (see Platform caveat); reproducible-build byte identity
   (cross-ref 04.02 — separately-marked xfail); argv passthrough (`sys.argv[1:]`
   in the binary = exactly the args after the binary name); cwd invariance (same
   bytes from two different working directories).
6. **Green-now half**: every expected triple in `cases/expected/` is re-verified
   against `/opt/homebrew/bin/python3.14` (3.14.7) on every run, and the harness
   self-comparison (`--impl python3.14`) is proven green over the exact 30-case
   slice, so the slice's reference bytes are known-good independent of 02.06.

## Testability contract under test

- Expose (02.06, verbatim): "cranelift-object pipeline, link static (musl for
  linux, standard for macos), embed ap-obj+ap-gc+reachable-stdlib-only (LTO
  dead-strip), emit `antipythany build app.py -o app`."
- Gate (02.06, verbatim): "`./app` runs a hello-world + a classes corpus program;
  binary <= 15MB musl; conformance suite runs against the BINARY, not the JIT, for
  a 30-case slice."
- Only documented surfaces are invoked: the Plan 04 verb table (`antipythany build
  app.py` -> `./app`, so `-o` names the output), and ap-conform's 00.04 CLI:
  `ap-conform run --corpus <glob> --impl <cmd>` (`<cmd> <file>` per case; `--filter`,
  `--json`, `--timeout`; reference is `python3.14` from PATH per C-01). No new CLI
  is proposed.
- Per-program artifacts vs `--impl`: ap-conform takes ONE impl command, but the
  binary is per-program (each corpus case is compiled separately). T-09 therefore
  expresses "the suite runs against the BINARY" as 30 single-file ap-conform runs,
  one freshly built binary per case (`--impl <path-to-that-binary>`). Recorded as
  G-5: the 00.04 CLI text does not describe a multi-program binary mode; if
  implementers later add one, this leaf's per-case form remains valid.
- Resolution order (mirrors T-05/T-06 conventions): CLI = env `APX_CLI`
  (whitespace-split) → `antipythany` on PATH → `target/{debug,release}/antipythany`
  under the repo root. Harness = env `AP_CONFORM` → `ap-conform` on PATH →
  `target/{debug,release}/ap-conform`. Reference = env
  `AP_CONFORM_REFERENCE_PYTHON`, default `/opt/homebrew/bin/python3.14`.
  Target mode = env `APX_TARGET` (`linux-musl` enables the 15MB assert).
- C-23 constraint honored: no test asserts finalizer/weakref timing; the gc
  slice cases are the existing property-style C-23-compliant corpus cases.

## Platform caveat — the "binary <= 15MB musl" clause

We develop on **macOS arm64**, where 02.06 links "standard for macos" — no musl
exists and the 15MB number was pinned for the linux-musl static link. T-09
therefore handles size as a **mode gate**:

- `APX_TARGET=linux-musl`: `test_binary_size_within_bound` hard-asserts
  `os.path.getsize(app) <= 15 * 1024 * 1024` (02.06 Gate verbatim).
- Default (macos): the same test **measures and reports** the size and asserts
  only that the artifact exists and is non-trivially small at JIT-stub scale
  (no byte bound is contractual for macos-arm64 — recording that no macos bound
  exists is G-2). CI on the 04.03 release matrix (linux-musl row) gets the real
  gate by setting `APX_TARGET=linux-musl`.

## Pinned 30-case slice (paths, repo-relative)

10 values + 10 classes + 5 exceptions + 5 gc = 30. Paths are pinned in
`SLICE_30` inside the suite; the manifest test asserts each exists.

```
tests/corpus/values/V001-bigint-exactness.py
tests/corpus/values/V003-floordiv-mod-signs.py
tests/corpus/values/V008-bool-is-int.py
tests/corpus/values/V013-slice-algorithm.py
tests/corpus/values/V014-fstring-formats.py
tests/corpus/values/V040-dict-insertion-order.py
tests/corpus/values/V047-truthiness-table.py
tests/corpus/values/V051-eq-vs-identity.py
tests/corpus/values/V061-float-repr-shortest.py
tests/corpus/values/V068-float-format-specs.py
tests/corpus/classes/c001_class_basics.py
tests/corpus/classes/c003_new_init_order.py
tests/corpus/classes/c008_data_descriptor_priority.py
tests/corpus/classes/c011_property_setter.py
tests/corpus/classes/c016_slots_messages.py
tests/corpus/classes/c019_mro_examples.py
tests/corpus/classes/c022_super_diamond_chase.py
tests/corpus/classes/c024_descriptor_kinds.py
tests/corpus/classes/c027_binop_dispatch.py
tests/corpus/classes/c033_isinstance_issubclass.py
tests/corpus/exceptions/e001_raise_requires_baseexception.py
tests/corpus/exceptions/e002_hierarchy_structure.py
tests/corpus/exceptions/e005_custom_hierarchy.py
tests/corpus/exceptions/e007_raise_bare_class.py
tests/corpus/exceptions/e015_exception_class_shape.py
tests/corpus/gc/gc001_del_runs_at_collect.py
tests/corpus/gc/gc002_finalizer_order_one_generation.py
tests/corpus/gc/gc003_refcycle_with_del.py
tests/corpus/gc/gc004_weakref_callback_on_collect.py
tests/corpus/gc/gc011_collect_returns_and_garbage.py
```

Rationale for the mix: values + classes carry the leaf's own gate examples
(hello-class programs); exceptions pins the traceback/rc path (C-25/C-29) that
a static binary must reproduce at shutdown; gc pins D1-shaped (timing-agnostic)
programs so the GC-embedded binary cannot crash or reorder finalizer output.
Byte anchors: the 25 non-gc cases ship recorded `expected/` files from sibling
leaves (T-05/T-06 conventions, 3.14.7); the gc cases' expected files were still
being generated by the T-07 session at authoring time (G-1), so the binary runs
go through ap-conform, which computes reference bytes live from `python3.14`
(00.04 contract) and needs no recorded files. All 30 were additionally
self-verified green through ap-conform `--impl python3.14` at authoring time.

## Inline exit-code cases (`tests/t09_binary/cases/`)

13 scripts; expected triples under `cases/expected/<stem>.{stdout,stderr,rc}`
recorded by REAL runs of `/opt/homebrew/bin/python3.14` (3.14.7, canonical
form: `python3.14 <name>.py` with cwd = `cases/`, argv extras for the probes;
traceback `File "..."` paths normalized to the literal `<PATH>` in x009/x012;
signal death recorded as the POSIX status `128+signo` = shell `$?`).

| File | Cites | Pinned (rc / stdout / stderr) |
| --- | --- | --- |
| x001_exit_int | C-28, S-06-11 | 2 / empty / empty |
| x002_exit_negative | C-28 | 255 / empty / empty (`sys.exit(-1)` verified) |
| x003_exit_str | C-28 | 1 / empty / `boom\n` |
| x004_exit_bare | C-28 | 0 / empty / empty |
| x005_exit_none | C-28 | 0 / empty / empty |
| x006_fall_through | C-28, C-30 | 0 / `ok\n` / empty |
| x007_exit_object | C-28 | 1 / empty / `custom-str\n` (raw `str()`, message not traceback) |
| x008_exit_list | C-28, S-06-11 | 1 / empty / `[1]\n` |
| x009_uncaught_valueerror | C-25, C-29, C-30 | 1 / `OUT1\n` / CPython-3.14 traceback (path-normalized), stderr only |
| x010_thread_systemexit | C-28 | 0 / `in-thread\nmain-done\n` / empty (thread exit code does NOT leak to process) |
| x011_argv_tail | 02.06, C-27 | 0 / `['alpha', '2']\n` — argv passthrough probe |
| x012_keyboard_interrupt | C-29, S-06-12 | 130 / empty / traceback ending `KeyboardInterrupt` (path-normalized) |
| x013_cwd_probe | C-04, C-27 | 0 / `cwd-ok\n` — cwd-invariance probe |

## Cross-references — 04.02 reproducibility

02.06 ships the binary; **04.02** owns build-cache and the reproducibility gate
("build output reproducible byte-for-byte across two clean caches ... linker
build-id - zeroed"). T-09 asserts two builds of the same source are byte-identical,
but as a **separately-marked xfail citing 04.02**: identity SHOULD hold only once
the linker build-id zeroing from 04.02 ships; a failure of THIS test alone does
not block 02.06 (the orchestrator tracks it in 04.02 instead). The xfail reason
string names `04.02` so gate runners can filter it.

## CONTRACT-GAPS (findings to file back per README §Protocol 5)

- **G-1 — gc expected files pending at authoring.** `tests/corpus/gc/expected/`
  was empty while the T-07 test session was still running. The five gc slice
  paths are pinned; their `.py` files exist and were re-verified deterministic
  (3 identical reference runs each, rc 0, no `0x` addresses) at authoring time.
  ap-conform computes reference bytes live, so the binary gate does not depend
  on the recorded files; only the file-based cross-check does. Re-run
  `test_ap_conform_harness_self_comparison_green_on_slice` when T-07 lands.
- **G-2 — no macos-arm64 size bound is contractual.** 02.06 names only
  "binary <= 15MB musl". On this host the link is "standard for macos" and any
  threshold would be invented. Resolution: mode-gated assert
  (`APX_TARGET=linux-musl`) + report-only on macos; 02.06/04.03 owners may pin a
  macos bound later, at which point the test gains one assert line (test gets
  STRONGER only — C-33).
- **G-3 — reproducible identity is 04.02 scope, not 02.06.** Included as a
  separately-marked xfail (see cross-reference section). If the orchestrator
  later decides 02.06's linker step should already zero build-id, flip that one
  test's marker from "awaits 04.02" to the 02.06 gate reason — do not merge the
  two markers.
- **G-4 — reference-build drift (shared caveat).** All triples recorded on
  `/opt/homebrew/bin/python3.14` = **3.14.7**; C-01 pins **3.14.0** and marks
  non-pinned-patch bytes provisional (same open item as 00-foundation PROGRESS.md
  and T-06 GAP-7). Re-verification is one ap-conform self-comparison run when a
  3.14.0 build lands; the green-now half of this suite re-measures the reference
  on every pytest run so drift is loud.
- **G-5 — ap-conform has no documented multi-program binary mode.** The 00.04
  CLI runs one `--impl` command over a glob; AoT produces one binary per program.
  T-09's per-case invocation form (`--impl <case's binary>`) is the minimal
  reading of "conformance suite runs against the BINARY ... for a 30-case slice"
  that uses only documented flags. If a batch mode is added later, the slice can
  collapse into one call; the 30 per-case tests stay valid either way.
- **G-6 — observed stub behavior (recorded, not gated).** At authoring time the
  scaffold `target/debug/antipythany build x.py -o out` exits 0 WITHOUT producing
  `out` (Plan 00.01 scaffold). T-09 treats "exit 0 but no artifact" as a build
  FAILURE of the seam (the test asserts existence, so it xfails rather than
  passing vacuously). C-07/C-11 establish "no artifact" as a valid refusal only
  alongside a nonzero exit; a zero exit with no artifact is undefined by the
  contract and should not be considered a shipped 02.06 interface.
- **G-7 — PATH-isolation semantics are implied, not written.** 02.06 says "single
  static binary" but no clause states the binary must run without any Python on
  PATH. T-09 asserts exactly that (minimal `PATH=/usr/bin:/bin`), because the
  master.md product promise is "native single static binaries (Go-style UX)".
  If an implementer believes the binary legitimately needs a python3.14 on PATH,
  that is a contract amendment (PR against compat-contract), not a test weakening.

## Per-test citations (suite `tests/t09_binary/test_t09_binary.py`)

| Test | Assertion | Cites | Status today |
| --- | --- | --- | --- |
| `test_expected_triples_reproduce` ×13 | reference re-run == recorded stdout/stderr/rc bytes (path-normalized) | S-06-11/12; C-28/C-29; C-01 caveat G-4 | GREEN |
| `test_inline_case_cites_contract` ×13 | every case file carries a `# Cites:` header line | tests/README.md corpus rules | GREEN |
| `test_slice_manifest_case_exists` ×30 | pinned slice path present under `tests/corpus/` | 02.06 Gate "30-case slice" | GREEN |
| `test_slice_composition_is_10_10_5_5` | 10 values / 10 classes / 5 exceptions / 5 gc | task contract | GREEN |
| `test_nonpending_expected_streams_clean` | the 25 recorded triples outside gc: rc 0, stderr empty | spec 06; C-24 | GREEN |
| `test_ap_conform_harness_self_comparison_green_on_slice` | `ap-conform run --corpus <case> --impl python3.14` rc 0 ×30 | 00.04 Gate; C-24; C-02 | GREEN |
| `test_harness_binary_resolvable` | ap-conform executable found via the documented resolution order | 00.04 CLI | GREEN |
| `test_build_hello_succeeds` | `antipythany build` rc 0 AND artifact exists + executable | 02.06 Expose | XFAIL (02.06) |
| `test_binary_runs_hello_program` | built hello binary stdout bytes == reference bytes | 02.06 Gate ("hello-world"); C-27/C-31 | XFAIL |
| `test_binary_runs_classes_corpus_program` | built c001 binary stdout bytes == recorded `.out` | 02.06 Gate ("classes corpus program") | XFAIL |
| `test_binary_runs_under_path_isolation` | run with `PATH=/usr/bin:/bin` (no python3.14 visible): identical 3-channel bytes | G-7; C-31; C-27 | XFAIL |
| `test_binary_exit_code_contract` ×12 | built x0NN binary reproduces pinned (rc, stdout, stderr) | C-28, C-29, C-30, S-06-11/12 | XFAIL |
| `test_ap_conform_slice_against_binary` ×30 | per case: build, then `ap-conform run --corpus <case> --impl <app>` rc 0 | 02.06 Gate verbatim ("against the BINARY, not the JIT"); G-5 | XFAIL |
| `test_ap_conform_whole_slice_against_binaries` | aggregate: all 30 binary runs pass | same | XFAIL |
| `test_binary_size_within_bound` | `APX_TARGET=linux-musl`: ≤15MB; else report-only on macos | 02.06 Gate; G-2 | XFAIL (no artifact) |
| `test_reproducible_build_byte_identity` | two clean builds of same source, `fileA == fileB` bytes | 04.02 Gate (linker build-id zeroing) | XFAIL (cites 04.02 separately) |
| `test_argv_passthrough` | `./app alpha 2` → stdout `['alpha', '2']\n` | 02.06 Expose UX; x011 | XFAIL |
| `test_cwd_invariance` | same binary, two cwds → identical bytes + rc | C-04, C-27; x013 | XFAIL |

## Run command

```
/opt/homebrew/bin/python3.14 -m pytest tests/t09_binary -q
```

From the repo root. Today: **89 passed, 50 xfailed, 0 errors** (139 total.
Green 89 = 13 triple re-verification + 13 citation checks + 30 slice-manifest
+ 1 composition + 1 recorded-streams-clean + 1 harness-resolvable + 30 slice
self-comparison. XFAIL 50 = 4 build/run/isolation + 11 exit-code + 30
ap-conform-vs-binary + 1 aggregate + 1 size + 1 reproducible (04.02) + 1 argv
+ 1 cwd). Overrides:
`APX_CLI=<cmdline>`, `AP_CONFORM=<cmdline>`,
`AP_CONFORM_REFERENCE_PYTHON=<path>`, `APX_TARGET=linux-musl`.

## xfail policy

- All antipythany-side tests are `xfail(strict=False)`: 02.06 is not implemented
  (the CLI is a silent stub), so bodies fail inside the XFAIL envelope.
- After 02.06 lands: cases run for real; XPASS is expected and reported; any
  FAIL is a gate failure for 02.06 — fix the implementation, never the test
  (master.md protocol; C-33). The reproducible-build test stays xfail-marked
  until 04.02's build-id zeroing ships (G-3).
- The green-now half must STAY green; a break there means the reference build
  drifted (C-01 / G-4) — that is a harness finding, not a 02.06 gate finding.

## Protocol notes

- Tests commit BEFORE 02.06 completes (README §Protocol 3); the gate runner is an
  agent that wrote neither these tests nor the codegen (§Protocol 4).
- No corpus file under `tests/corpus/` was modified; the slice references them
  read-only. New files are confined to `tests/t09_binary/`.
