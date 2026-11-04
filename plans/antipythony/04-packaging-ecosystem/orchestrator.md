# Plan 04 - Packaging and Ecosystem

Goal: the user experience. `antipythony new/run/build`, uv-backed dependency
resolution from v0, build cache, release engineering, and all three distribution
shapes (native, wasm, hybrid).

## Design

CLI surface (clap, ap-cli crate):

```
antipythony new myapp           # scaffold: myapp.py + antipythony.toml
antipythony add requests        # resolve+lock via uv, record in antipythony.toml
antipythony run app.py          # compile (cached) + execute native
antipythony run --wasm app.py   # compile to wasm + execute via ap-host
antipythony build app.py        # -> ./app          (native static binary)
antipythony build --wasm app.py # -> ./app.wasm     (portable module)
antipythony build --hybrid      # -> ./app (native w/ embedded wasm payload)
```

Dependency model: antipythony.toml (project deps) + antipythony.lock. Resolution and
fetching shell out to uv (`uv pip compile` + `uv pip download` to a local cache dir)
- subprocess use, never a library dependency (uv is standalone Apache-2.0; subprocess
keeps our binary clean). Only pure-Python wheels are accepted in v1; a wheel with
native components produces the C-extension diagnostic from the compat contract at
ADD time, not build time (fail early).

Build cache: `~/.cache/antipythony/build/<hash>` keyed by (input hash, lockfile
hash, toolchain version, backend, opt level). Content-addressed; `run` is warm in
milliseconds after first build.

## Leaves

### 04.01 - uv integration
toml+lock schema; uv subprocess wrapper (detect uv, pin minimum version, helpful
error if absent); pure-wheel-only acceptance filter; the native-component
diagnostic at `add` time. Gate: `new`/`add`/`run` flow with one pure-Python dep
(e.g. `rich`) works end to end on a clean cache.

### 04.02 - ap-cli verbs + build cache
All verbs per the surface above; content-addressed cache with the key schema above;
`run` uses cache; `--no-cache`; cache GC subcommand. Gate: second `run` of same
input <200ms to execution; `build` output reproducible byte-for-byte across two
clean caches (reproducibility gate, documented exceptions: linker build-id - zeroed).

### 04.03 - Release engineering
CI matrix (linux-musl static, linux-gnu, macos-arm64, macos-x86_64), release
artifacts + checksums, THIRD_PARTY_NOTICES embedded in binaries and emitted by
`--version --notices`, install script. Gate: fresh-machine install -> new -> add
rich -> run -> build, scripted, green.

### 04.04 - Wasm distribution path
`build --wasm` plumbed to Plan 03 output; a `ap-host` standalone runner binary
shipped alongside (users without wasmtime can `./ap-host app.wasm`); doc page
"run your .wasm anywhere" listing browser/wasmtime/node paths. Gate: built wasm runs
under both shipped ap-host and system wasmtime.

### 04.05 - Hybrid artifact
Native binary with the wasm module embedded (payload section / include_bytes at link
time); `app --emit-wasm` extracts; embedded ap-host execution path behind a flag.
Gate: one binary passes native conformance slice AND its extracted wasm passes the
wasm slice.

## Dependencies
04.02 needs 02.06 (native build verb works); 04.04/04.05 need Plan 03.

## Dispatch notes
04.01 and 04.02 can start as soon as 02.06 lands (CLI + cache around native builds);
04.03 after 04.02; 04.04/04.05 after Plan 03.

## Test Gate

<!-- Tests-first gate (hierarchical-planning skill v1.3.0, patched 2026-10-24).
     Coverage matrix derived mechanically from leaf task headers; the control
     agent (orchestrator, in-session) refines rows and fills test names at
     Phase 0 dispatch. No implementation leaf dispatches until every row is
     VALIDATED. Delta protocol: lesser of plan/tests updates to the greater.
     Recipe: skill hierarchical-planning -> references/tests-first-gate.md -->

| # | Feature (from plan) | Source | Unit tests | E2E tests | Status |
|---|---------------------|--------|-----------|-----------|--------|
| F1 | Plan 04 progress log | PROGRESS.md | (pending) | (pending) | PENDING |

Statuses: PENDING | DELTA | VALIDATED

Delta log: (none yet)
