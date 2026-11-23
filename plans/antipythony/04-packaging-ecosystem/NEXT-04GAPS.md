# NEXT-04GAPS — Plan 04 honest-gap closeout session record (2026-11-22)

Branch `tier1-04gaps`, worktree `~/.git/worktrees/leaf-04gaps`, off main
`8c8bfca`. Closes the four gaps listed in NEXT-0405.md §"What Plan 04 /
follow-ups should pick up". Probe-first: every gap reproduced with exact
bytes BEFORE the fix, re-probed green after.

## Verdict

**ALL FOUR GAPS CLOSED.** Gates: `cargo test -p ap-cli -p ap-conform`
19 suites, 86 passed / 0 failed (baseline inherited 85 pass + the one
pre-existing fence failure caused by the missing `target` symlink /
release probe — fixed per the NEXT-0405 gotcha, not a code bug);
ap-aot-crt 3/3, ap-aot-crt-hybrid 4/4; hybrid emit-wasm end-to-end
green; ap-conform --impl-wasm 26/26 byte-exact vs CPython 3.14.7;
plain-path hello byte-identical vs CPython 3.14.7.

## Gap 1 — plain-CRT --emit-wasm loud diagnostic (f06c293)

- `crates/ap-aot-crt/src/lib.rs`: the plain CRT now claims EXACTLY
  `--emit-wasm` / `--wasm` as argv[1] BEFORE manifest registration and
  fails with the D3 actionable shape (rc 2, stderr names the fix).
- BEFORE (probe bytes, argv-ignoring probe): stdout
  `hello ['--emit-wasm']\n`, rc 0, no .wasm (the flag silently became
  program argv). `--wasm` identical shape.
- AFTER (exact bytes): stdout EMPTY, stderr
  `antipythony: --emit-wasm: this binary was built with plain \`build\` (no wasm payload seam) — rebuild with \`antipythony build --hybrid <file.py>\` to embed the wasm module and claim --emit-wasm`
  rc 2, no .wasm. Same for `--wasm`.
- Trivially-fixable-vs-diagnostic audit: extraction cannot work on a
  plain binary BY CONSTRUCTION (no payload symbols; nm-proven by the
  04.05 gate), so the diagnostic is the only honest fix — chose it.
- `crates/ap-cli/tests/hybrid_gate.rs`
  `plain_binary_treats_emit_wasm_as_program_argv` RE-PINNED: it pinned
  the OLD passthrough behavior (its own comment said the refusal was
  another agent's surface); now asserts stdout empty, rc 2, stderr
  names flag + `build --hybrid`, no .wasm.

## Gap 2 — `--` separator for the hybrid flag seam (b697d30)

- `crates/ap-aot-crt-hybrid/src/lib.rs`: a leading `--` suppresses the
  flag claim entirely — the program receives every argv entry after it
  verbatim on the native path (registration + exit mapping unchanged).
- BEFORE: hybrid binary whose first program-argv should be `--wasm` had
  no way to receive it (the claim swallowed it; probe `./app --wasm`
  routed to the embedded host, program never saw the flag).
- AFTER probes: `./app -- --emit-wasm` → program runs, rc 0, ZERO .wasm
  written (claim suppressed — the sharp marker); `./app --emit-wasm`
  still claims (rc 0, cf001.wasm written); bare `./app --` stays on the
  native path with argv intact. Unit test pins SEP == "--" and that it
  is not in the claim set.
- Design note: CPython keeps `--` itself in sys.argv
  (`python s.py -- --wasm` → argv `[s.py, --, --wasm]`); here the
  launcher consumes `--` (it is OUR interpreter-flag seam, not the
  program's), so `./app -- --wasm` gives the program argv
  `[app, --wasm]` — the convention CPython uses for its own `-X`
  style options, recorded deliberately.

## Gap 3 — ap-conform `--impl-wasm` (b63d2cd)

- `crates/ap-conform/src/main.rs`: `--impl-wasm '<prefix>'` mirrors
  `--impl-binary` (02.06 T5 shape): per case, `<builder> build --wasm
  <case> -o <tmp>/conform-<n>.wasm>` (builder = `--impl-wasm-builder`,
  default: the `antipythony` binary next to ap-conform), then the impl
  prefix consumes the MODULE path. Emitter refusal → ERROR outcome
  (fail-closed, never a silent skip).
- Gate (executed, PATH with target-04gaps/debug for ap-host):
  controlflow_wasm 18/18 PASS, classes_wasm 8/8 PASS (byte-exact vs
  CPython 3.14.7); `probes/imptest.py` (`import sys`) → ERROR with the
  wasm-build refusal recorded. This is the harness form T-16's
  `test_ap_conform_wasm_slice_via_ap_host_impl` xfailed for — the xfail
  leg can be flipped by the T-16 track (separate authorship; not
  touched here).

## Gap 4 — hybrid CRT packaging for the 04.03 release matrix (d7dd012)

- `crates/ap-cli/src/build.rs`: `crt_staticlib_hybrid` refactored into
  `crt_staticlib_hybrid_from(exe_path, target)`; probes
  `$PREFIX/lib/antipythony/libap_aot_crt_hybrid.a` FIRST (the D3
  layout), then the historical chain. Not-found error names the install
  path + scripts/install.sh. Unit test `hybrid_crt_lookup_finds_staged_
  install_prefix_crt` mirrors the D3 test (positive + negative arm).
- `scripts/package-release.sh`: packs the hybrid staticlib (BIN_DIR
  first, then `target*/release`); missing → LOUD warning.
- `scripts/install.sh`: installs it at
  `$PREFIX/lib/antipythony/libap_aot_crt_hybrid.a`.
- `docs/packaging.md`: new "Hybrid CRT packaging" section (layout,
  lookup order, loud limitation).
- **RELEASE.YML — NOT edited (another agent's file). EXACT suggested
  diff** for `.github/workflows/release.yml` Build step (after the
  `ap-aot-crt` line ~118):

```yaml
          cargo build --release --target ${{ matrix.target }} -p ap-aot-crt
+         # 04.05 gap-4: the hybrid CRT ships so installed binaries can
+         # `build --hybrid` (package-release.sh packs it; install.sh
+         # places it at $PREFIX/lib/antipythony/libap_aot_crt_hybrid.a).
+         cargo build --release --target ${{ matrix.target }} -p ap-aot-crt-hybrid
```

  Until that lands, the archive warns loudly and `build --hybrid` from
  an installed binary fails with the actionable not-found diagnostic —
  never silent half-support.

## Gates (executed, not claimed)

- Baseline before my changes: the same `cargo test -p ap-cli -p
  ap-conform` run had ONE failure (`ast_getattr_fence` — spawn-NotFound,
  the documented fresh-worktree gotcha; fixed by `ln -sfn target-04gaps
  target` + release probe build per NEXT-0405 gotcha 5, NOT by a code
  change). Inherited suite counts: 19 suites / 85 passed incl. that
  fence fail; final: **19 suites, 86 passed, 0 failed** (I grew
  ap-aot-crt +1, ap-aot-crt-hybrid +1, ap-cli lib +1 tests).
- `cargo test -p ap-aot-crt -p ap-aot-crt-hybrid` → 3/3 + 4/4.
- End-to-end: hybrid build + `--emit-wasm` (cf001.wasm, rc 0, magic
  `\0asm` verified by the 04.05 gate suite re-run green); plain build
  NEW diagnostic (exact bytes above); ap-conform --impl-wasm 26/26.
- Gate 3: `antipythony build app.py && ./app` hello stdout
  `hello, world\n` **byte-identical vs CPython 3.14.7** (cmp clean,
  stdout + stderr + rc 0 both sides).

## Machine policy

ONE heavy cargo lane; CARGO_BUILD_JOBS=4; all worktree/logs under
`~/.git/worktrees/leaf-04gaps/` (gates-04gaps.log,
gates-04gaps-final.log); probes/ scratch (untracked); explicit
pathspecs only (never `git add -A`); main checkout untouched.

## Files

- crates/ap-aot-crt/src/lib.rs (gap 1)
- crates/ap-aot-crt-hybrid/src/lib.rs (gap 2)
- crates/ap-conform/src/main.rs (gap 3)
- crates/ap-cli/src/build.rs + scripts/{install,package-release}.sh +
  docs/packaging.md (gap 4)
- crates/ap-cli/tests/hybrid_gate.rs (gap 1 re-pin)
- plans/antipythony/04-packaging-ecosystem/NEXT-04GAPS.md (this record)
