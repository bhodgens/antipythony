# T-14 — cli/cache (test leaf)

**Gates implementation leaves:** 04.01 (uv integration) + 04.02 (ap-cli verbs + build cache)
**Written from:** `plans/antipythony/04-packaging-ecosystem/orchestrator.md` (CLI surface block, Dependency model, Build cache key schema, 04.01/04.02 Expose+Gate), `docs/compat-contract.md` (C-06..C-10 fail-loud + APX tokens, C-26 output bytes), `docs/spec/05-builtins-stdlib-v1.md` (purity rules S-05-1..3), `plans/antipythony/master.md`, `plans/antipythony/tests/README.md`. No `crates/*/src` read (separation rule).

## Purpose

Pin the observable contract of the packaging UX before it is built: the CLI verbs
(`new` / `add` / `run` / `run --no-cache` / `build` / `cache gc`), the
`antipythony.toml` + `antipythony.lock` schema, the pure-wheel-only filter firing
at **add time** with the C-07/C-08/C-10 extension diagnostic, the content-addressed
build cache under `~/.cache/antipythony/build/<hash>` with its five key dimensions,
the missing-uv helpful error, and byte-reproducible `build` across clean caches.

## Testability contract (leaf Expose/Gate citations)

| Leaf | Expose (what must be callable/inspectable) | Gate (success bar) | How T-14 tests it |
| --- | --- | --- | --- |
| 04.01 | "toml+lock schema; uv subprocess wrapper (detect uv, pin minimum version, helpful error if absent); pure-wheel-only acceptance filter; the native-component diagnostic at `add` time" | "`new`/`add`/`run` flow with one pure-Python dep (e.g. `rich`) works end to end on a clean cache" | `test_new_scaffolds_project_files`, `test_add_local_pure_dep_writes_lock` (offline `file:` variant of the flow), `test_add_uv_registry_flow_pypi_offline_skip` (verbatim `rich` variant, network-gated), `test_add_native_wheel_rejected_at_add_time_with_cext_diag`, `test_add_without_uv_on_path_errors_helpfully`. Clean cache = isolated `HOME` per test (`_clean_env`). |
| 04.02 | "All verbs per the surface above; content-addressed cache with the key schema above; `run` uses cache; `--no-cache`; cache GC subcommand" | "second `run` of same input <200ms to execution; `build` output reproducible byte-for-byte across two clean caches (documented exceptions: linker build-id — zeroed)" | `test_run_cold_then_warm_same_cache_entry`, `test_run_no_cache_bypasses_cache`, `test_cache_survives_across_processes`, `test_cache_key_changes_with_input_hash`, `test_cache_key_changes_with_opt_level`, `test_cache_gc_verb`, `test_build_reproducible_byte_identical_across_clean_caches`, `test_run_wasm_verb_plumbing`, `test_warm_run_timing_is_measured_and_reported`. |

Contract clauses cited across the suite: **C-06** (third-party deps enter scope only via 04.01; same extension rule applies at add), **C-07/C-09/C-11** (refusal is total, no stub lock entry after a refused add), **C-08/C-10** (APX-CEXT token + full module name + `C extension` + policy pointer + file:line:col asserted on the `add` verb's stderr), **C-26** (cold/warm/`--no-cache` run output bytes identical), **S-05-1..3** (fixture purity checks), Plan-04 §Design lines verbatim for cache layout and key dims.

## Measured-vs-asserted decisions (the <200ms gate)

- **<200ms to execution — MEASURED, REPORTED, NOT ASSERTED** (G-3). The leaf's
  figure is a UX intent, not a deterministic machine bound: process spawn,
  Python-import of the app, and terminal timing noise make a hard ms assert
  flaky, and T-08's G-2 precedent (size/latency figures reported, contractually
  asserted only where a bound is machine-checkable) applies. The test prints
  `T-14 WARM-RUN REPORT: ... ms`.
- **What IS asserted instead** (the deterministic core of the same gate):
  cache-hit determinism (warm run adds **zero** new cache entries vs cold),
  cross-process cache survival (separate subprocess invocations share entries,
  no duplication), `--no-cache` bypass (entry set unchanged while the program
  still executes), key-dimension sensitivity (changed input / opt level ⇒ new
  entry, and the edited program's output proves freshness), and real execution
  bytes on every run path (`b"hello, antipythony"`), so a silent exit-0 stub
  cannot XPASS an invariance-only assertion — this caught 3 vacuous XPASSes
  during authoring against the pre-existing `target/debug/antipythony` stub.
- **Reproducible build — asserted** (byte-for-byte across two clean-HOME
  caches), with the documented exception per the leaf: linker build-id is
  zeroed, so the assert is raw-bytes equality with no build-id carve-out;
  if an implementation leaves build-id nonzero the test fails loudly and the
  exception belongs in a contract amendment, not a test weakening.

## uv availability note (measured at authoring, 2026-10-16)

`uv 0.11.29 (901092ee1 2026-07-15 aarch64-apple-darwin)` present at
`/Users/caimlas/.local/bin/uv`. uv is a real, installable external tool and IS
here, so the subprocess half is **not** pre-skipped — only the `antipythony`
binary's absence drives xfail. `test_uv_availability_report` records the live
version; the leaf says "pin minimum version" without naming the pin (G-5).

## Self-containment / network policy

No test fetches from a registry as its primary path. The `add` fixture is the
local pure-Python package `fixtures/tiny_pure` (installed via `file:` path).
The one PyPI-anchored test (`add rich`) probes DNS first and
`pytest.skip`s with reason when offline (G-7). If a *local* add still fails
because uv reaches out for build-backend packages, the test detects the network
error text and skips-with-reason rather than falsely failing the leaf. The
native-component wheel fixture is **built in-test** (zipfile) so no binary blob
is committed.

## Per-test citation table

| Test | Half | Status | Citations |
| --- | --- | --- | --- |
| `test_expected_toml_fixture_parses` | oracle | green | 04.01 "toml+lock schema"; Plan-04 §Design toml; G-1 |
| `test_expected_lock_fixture_parses` | oracle | green | 04.01 schema; §Design "resolve+lock via uv"; G-1 |
| `test_fixture_tiny_pure_is_self_contained_package` | oracle | green | 04.01 Gate; S-05-1..3; C-01 (3.14) |
| `test_app_seed_only_imports_fixture_dep` | oracle | green | §Design "scaffold: myapp.py + antipythony.toml" |
| `test_native_wheel_fixture_builds_and_is_detectably_impure` | oracle | green | 04.01 "pure-wheel-only acceptance filter"; C-06 |
| `test_cext_diagnostic_expectation_matches_c08_regex` | oracle | green | C-08 Verify regex; C-10 APX-CEXT; C-07 |
| `test_cache_root_layout_constant` | oracle | green | §Design "Build cache: ~/.cache/antipythony/build/<hash>"; G-2 |
| `test_uv_availability_report` | oracle | green | 04.01 "detect uv, pin minimum version"; G-5 |
| `test_new_scaffolds_project_files` | 04.01 | xfail | surface `new`; 04.01 toml schema; G-1 |
| `test_add_local_pure_dep_writes_lock` | 04.01 | xfail | 04.01 Gate (offline `file:` variant); §Design lock; G-6/G-7 |
| `test_add_uv_registry_flow_pypi_offline_skip` | 04.01 | xfail | 04.01 Gate verbatim (`rich`); G-7 network policy |
| `test_add_native_wheel_rejected_at_add_time_with_cext_diag` | 04.01 | xfail | 04.01 "diagnostic ... at ADD time, not build time"; C-07/08/09/10/11 |
| `test_add_without_uv_on_path_errors_helpfully` | 04.01 | xfail | 04.01 "helpful error if absent"; G-4 |
| `test_run_cold_then_warm_same_cache_entry` | 04.02 | xfail | 04.02 "run uses cache"; §Design key; C-26; G-2/G-3 |
| `test_run_no_cache_bypasses_cache` | 04.02 | xfail | surface `run --no-cache`; 04.02 Expose |
| `test_cache_survives_across_processes` | 04.02 | xfail | §Design content-addressed cache; design req "cache survives across processes" |
| `test_warm_run_timing_is_measured_and_reported` | 04.02 | xfail | 04.02 Gate "<200ms" (MEASURED per G-3) |
| `test_cache_key_changes_with_input_hash` | 04.02 | xfail | key dim "input hash" |
| `test_cache_key_changes_with_opt_level` | 04.02 | xfail | key dim "opt level"; G-2 (no flag specced — toml fallback probed) |
| `test_cache_gc_verb` | 04.02 | xfail | 04.02 "cache GC subcommand"; G-8 eviction policy |
| `test_build_reproducible_byte_identical_across_clean_caches` | 04.02 | xfail | 04.02 Gate reproducibility (build-id-zeroed exception); cross-ref T-09 GAP-3 |
| `test_run_wasm_verb_plumbing` | 04.02 | xfail | surface `run --wasm` (full runner = 04.04; this pins verb+output only) |

## How to run

```
/opt/homebrew/bin/python3.14 -m pytest tests/t14_cli -q          # 8 passed, 14 xfailed
/opt/homebrew/bin/python3.14 -m pytest tests/t14_cli --collect-only -q  # 22 collected, 0 errors
```

CLI resolution honours `APX_CLI=<cmdline>` (T-09 convention). Beware: a stale
`target/debug/antipythony` stub is auto-discovered; the run-half asserts real
program bytes so a do-nothing stub cannot turn xfail into xpass.

## CONTRACT-GAPS (filed back against the plan/spec text — do not silently resolve)

- **G-1 toml/lock field names unpinned.** 04.01 says "toml+lock schema" but
  names no fields. `fixtures/expected_antipythony.{toml,lock}` freeze a minimal
  derived schema (`[project] name/version/requires-python`, `[build]
  backend/opt_level`, `[[dependency]] name`, lock `[metadata] created_by` +
  `[[package]] name/version`). Implementation choosing different names is a
  contract miss to reconcile at integration, not a test bug. The add-flow test
  deliberately asserts *structurally* (name substring + non-empty pin) so it
  survives prose-level renames while the schema tests state the expectation.
- **G-2 cache hash-input recipe unpinned.** Layout `~/.cache/antipythony/build/
  <hash>` and the five key dims (input hash, lockfile hash, toolchain version,
  backend, opt level) are given; *how* they hash into `<hash>`, the entry dir
  layout inside it, and which CLI flags surface backend/opt-level are not.
  Tests therefore only assert entry-set *behaviour* (stability/inequality),
  never hash values or dir internals.
- **G-3 <200ms figure untestable as assert.** See measured-vs-asserted above;
  "to execution" is also undefined (spawn? first stdout byte?). Report-only.
- **G-4 verb error texts unspecced.** "helpful error if absent" has no
  wording/token spec. Minimal claim tested: nonzero exit + stderr mentions
  `uv`. If a stable token (à la APX-*) is wanted for this class, it needs a
  contract amendment (C-10 currently tokens compile errors only).
- **G-5 uv minimum version unpinned.** 04.01 says "pin minimum version" without
  saying which. Report-only test.
- **G-6 `add <local path>` form unspecified.** Surface shows `add requests`
  (registry). The offline test assumes `file:<path>` (uv convention) and a bare
  path fallback by error-skip; if the CLI accepts neither, offline e2e coverage
  is impossible and the Gate's "clean cache" wording (04.01) implies network anyway.
- **G-7 network policy vs Gate.** 04.01 Gate's `rich` end-to-end needs PyPI
  (uv resolves+downloads). Test track mandates self-containment; the registry
  test skip-with-reason when offline is the compromise. Orchestrator should run
  it once online for the gate.
- **G-8 cache GC semantics unspecced.** Verb existence only; no eviction
  policy (LRU? lockfile-scoped reachability?) in any read document.
- **G-9 reproducible-build exception scope.** The leaf's exception covers the
  *linker build-id*; not addressed: embedded timestamps in the lockfile-hash
  dimension, or uv-resolution order across mirrors. Byte-equality assert is
  kept whole; any second exception class needs a contract amendment.
- **Observation (not a gap):** `target/debug/antipythony` exists as a stub
  during authoring; three invariance-only assertions XPASSed vacuously until
  real-execution byte checks were added. Downstream test authors on this plan:
  assert program output bytes on every CLI path, never exit-code invariance alone.
