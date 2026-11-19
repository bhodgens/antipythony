# Run your .wasm anywhere

Leaf 04.04. You built a module with the Plan 03 wasm backend:

```console
$ antipythony build --wasm app.py        # -> ./app.wasm (portable WasmGC module)
```

The output is a single `.wasm` file — no runtime payload inside it, nothing
machine-specific. It embeds the source file NAME for tracebacks (the invoked
filename, CPython-faithful) and carries the `aprt-version` custom-section
stamp (payload `0`). The supported language slice is the Plan 03 corpus slice
(control flow, functions, classes, exceptions, generators, match — 51/51
byte-exact vs CPython 3.14 through the CLI); anything outside the slice fails
the BUILD loudly (`Unsupported ...`) — never a silently-wrong module.

The module imports exactly three things:

| import | signature | role |
|---|---|---|
| `wasi_snapshot_preview1.fd_write` | `(i32,i32,i32,i32) -> i32` | stdout/stderr (reads iovecs from the exported `memory`) |
| `aprt.abort_msg` | `(anyref)` | unrecoverable internal error (payload is a `$str`; hosts render it) |
| `wasi_snapshot_preview1.proc_exit` | `(i32)` | exit code |

It exports `memory`, `_start`, and the `aprt_version` global. That is the
whole ABI — every runner below is a thin shim over it.

## 1. ap-host — the shipped runner (recommended)

`ap-host` is the standalone runner binary from `crates/ap-host` (wasmtime 47
underneath). Users without wasmtime installed run:

```console
$ cargo build -p ap-host                  # release recommended (see startup note)
$ ./target/release/ap-host run app.wasm   # `run` is optional sugar
hello, world
$ echo $?
0
```

Flags (default-deny capability model — args/env are allowlists, the host
environment is never inherited):

```
--cap <bytes>     capture pipe bound (default 262144)
--arg <a>         grant one module arg (repeatable; bare positionals after
                  the .wasm path ride the same grant)
--env K=V         grant one env var (repeatable; default deny)
--argv0 <name>    argv[0] presented to the module
--ttfb            print time-to-first-stdout-byte to stderr
--validate-only   surface checks + compile, no execution
```

Exit codes: the module's own code on `proc_exit`; **126** = surface/ABI/
threading refusal (missing or future `aprt-version` stamp, shared memory,
unknown import); **127** = host/internal error.

Startup: hello-world ttfb medians ~39ms on the release build (the <50ms
budget from Plan 03 holds on release; debug builds are ~750ms — cranelift
debug overhead — and are for development only).

**Where to get the binary:** today it builds from this repo
(`cargo build -p ap-host --release`). The release-engineering leaf (04.03)
adds it to the per-platform release artifacts + checksums alongside
`antipythony`; that is the distribution decision RECORDED here: ap-host
ships as its own binary in the release matrix, and `antipythony host`
(below) covers users who only have the main CLI.

## 2. `antipythony host` — no separate binary

The main CLI embeds the same ap-host library and engine:

```console
$ antipythony host app.wasm                     # == ap-host run app.wasm
$ antipythony host app.wasm --arg x --env K=V   # same capability flags
```

Same byte-exact stdout/stderr contract, same rc propagation, same 126/127
refusal codes (forwarded through `ap_host::run_with_options`). Convenience
only — the standalone binary is the canonical shipped runner.

## 3. `antipythony run --wasm` — build + run in one step

```console
$ antipythony run --wasm app.py     # compile (in-memory temp) + execute
```

Uses the same embedded host; no `.wasm` file is left behind. Note: the
04.02 content-addressed build cache does NOT cover the wasm backend yet
(its keys are native-backend-shaped); a wasm cache key is future work and
documented in `docs/packaging.md`.

## 4. System wasmtime

The module runs unmodified on the stock `wasmtime` CLI (v47+ — anything
with WasmGC + exception-handling + tail-calls support, all stable in
wasmtime since ~23):

```console
$ wasmtime run app.wasm
hello, world
```

wasmtime supplies the WASI p1 imports natively; `aprt.abort_msg` is not
part of WASI, so only modules that never hit the internal-error path run
unmodified this way (the whole green corpus qualifies — `abort_msg` never
fires on it). This leaf VERIFIED the engine-level contract (the same
engine wasmtime 47 pinned 51/51 byte-exact vs CPython in the Plan 03 CLI
corpus gate) but the system-CLI arm was NOT exercised on the leaf machine:
no `wasmtime` CLI present (checked `which wasmtime`; installing toolchains
is a user decision, recorded honestly in
`plans/antipythony/04-packaging-ecosystem/NEXT-0404.md`). If you have the
CLI, the smoke is one command.

## 5. Browser (native WasmGC)

wasmtime does not run in browsers (no wasm32-unknown-unknown build —
decision recorded in leaf 03.06). Instead the module runs DIRECTLY on the
browser's native WasmGC + exception-handling + tail-calls support through
a ~60-line JS shim providing the three imports. `examples/browser-smoke/
index.html` is a self-contained demo embedding three corpus modules (the
EXACT bytes ap-host runs) and a live shim; verified byte-exact in Chrome
157 (2026-11-18). Firefox 129+ should work (GC+EH+tail-calls all stable
there) but was not verified on this machine — the demo page says so.

To embed your own module: build it, serve the page from any static server,
and swap the base64 blob — the shim needs only the exported `memory`.

Node.js: not wired (option (c) of the 03.06 brief, skipped as unnecessary
when the browser path worked). Node 22+ has WasmGC; the same shim logic
ports directly if you need it.

## Porting a runner (the whole contract)

A host must: (1) provide the 3 imports above, (2) call `_start` with no
args, (3) map a `proc_exit(n)` trap to exit code n, (4) render the
`abort_msg` `$str` payload if it fires (kind 0 latin1 bytes / kind 1 UTF-16
/ kind 2 UTF-32 — see `crates/ap-host/src/str_reader.rs`). Modules are
refused pre-compile when the `aprt-version` stamp is missing or newer than
the host supports (ABI drift fails fast, never an instantiate-time
unknown-import trap).

## Honest limits

- The wasm backend serves the Plan 03 slice. `import` of stdlib modules,
  file I/O, threading (no wasi-threads; wasmtime 47 removed the proposal
  layer), and sockets are out — a build touching them fails loud with the
  construct named.
- `run --wasm` bypasses the build cache (native-shaped keys only today).
- wasmtime CLI arm of the gate: recorded, not executed on this machine
  (absent; no install without user OK). The engine is the SAME wasmtime
  47 both shipped runners embed, which IS the Plan 03 51/51-verified one.
