# Wave-110 — scrapy crawl chain (T1)

## Summary

The wave-109 `'ModuleNotFoundError: No module named cryptography'` blocker is
**CLEARED** — the full `scrapy.extensions.telnet → twisted.conch →
twisted.conch.ssh → cryptography.hazmat.*` import chain now succeeds, along
with every module-level cryptography import in the staged tree (probed
exhaustively). Four additional real bugs were found and fixed en route. The
crawl **advances further than any prior wave**: the reactor installs, the
resolver binds, the asyncio loop spins Tasks — and stops at the *next*
blocker: **v0 coroutine stubs never execute their bodies**
(`async def` functions return inert marker objects; `send()` always raises
`StopIteration('')`), which is a `generators.rs`/`interp.rs` fix (sibling-
owned, see Finding W110-5).

## Finding W110-1: cryptography import chain — named + shimmed

**Entry point audit** (deliverable 1): scrapy's ExtensionManager eagerly
imports `scrapy.extensions.telnet` (module level
`from twisted.conch import telnet` + `from twisted.conch.insults import
insults`) regardless of `TELNETCONSOLE_ENABLED=False`. `twisted.conch.telnet`
does NOT import conch.ssh — the ssh pull is
`scrapy.utils.engine → …`/middleware import surface reaching
`twisted.conch.ssh.{keys,common,transport,_kex}` (keys.py imports
`cryptography.hazmat.primitives.{serialization,asymmetric.*}` broadly;
common.py imports `cryptography.utils.int_to_bytes`; transport/_kex import
`cryptography.hazmat.primitives.ciphers`). TELNET itself is a red herring —
the imports are eager-but-unused for the file:// gate, so bypassing was not
viable; the shim was the correct cheap fix.

**Surface pinning** (deliverable 2): probed live CPython 3.14
(`/tmp/probe_crypto_surface.py`, output `/tmp/crypto_surface.json`) for every
module in the import closure. 20 modules enumerated.

**Shim** (deliverable 3): `vendored_stdlib/cryptography/` — 24 files:
`__init__.py` (classes + `int_to_bytes` REAL implementation — it is pure
SSH-framing arithmetic and twisted.conch.ssh.common calls it at runtime),
`utils.py`, `exceptions.py`, `hazmat/{__init__,backends}` ,
`hazmat/primitives/{__init__,hashes/,serialization/,ciphers/}`,
`hazmat/primitives/asymmetric/{__init__,dsa,ec,ed25519,rsa,padding,dh,
x25519,utils}.py`, `hazmat/decrepit/ciphers/{__init__,algorithms.py}`.
Design notes:
- `hashes`/`serialization`/`ciphers.algorithms`/`ciphers.modes` are PACKAGES
  (dirs with `_mod.py` holding classes) because CPython's
  `from cryptography.hazmat.primitives import hashes` requires a real
  submodule; a flat module whose classes shadow the module name breaks.
- `ec.SECP256R1` etc. are **callable singletons** — twisted keys.py does
  `ec.SECP256R1()` at module level.
- All crypto ops raise `NotImplementedError` lazily (OpenSSL/SSL.py pattern).
- 14/14 CPython probes + 8/8 antipythany probes green
  (`/tmp/probe_crypto_shim.py`, `/tmp/probe_crypto_ap.py`).

## Finding W110-2: `time.localtime/gmtime(float)` TypeError (logging blindness)

**Every scrapy log record died in `Formatter.formatTime`** —
`time.localtime(record.created)` with float epoch raised
`TypeError: 'float' object cannot be interpreted as an integer`. The crawl's
"Logging error" storms (6 per run, since wave-100) were this bug eating ALL
log output AND the real tracebacks. CPython accepts floats (rounds via
time_t). **Fixed** in `builtins_modules.rs` (owned): `gmtime`/`localtime`
arms accept `Value::Float` via `f.round() as i64`. Corpus + differential
gates re-verified green after the change.

## Finding W110-3: `itertools.count(1).__next__` not callable

`asyncio/tasks.py` module level `_task_name_counter =
itertools.count(1).__next__` — the runtime's Iterator attr surface
(interp.rs:11007, sibling-owned) returns the iterator OBJECT for `__next__`
instead of a bound method, so `c.__next__()` → "'iterator' object is not
callable". Every `Task.__init__` died. **Fixed Python-side**
(behavior-identical): `_TaskNameCounter` class in `vendored_stdlib/asyncio/
tasks.py` with identical "Task-N" naming. The interp.rs arm should
eventually return a bound native-method marker (finding recorded for the
sibling lane).

## Finding W110-4: `sys.get_asyncgen_hooks` missing

`BaseEventLoop._run_forever_setup` calls `sys.get_asyncgen_hooks()` +
`sys.set_asyncgen_hooks(...)` — neither exists in the runtime's sys surface.
**Fixed Python-side**: hooks no-op'd in `base_events.py` (debug/finalization
aid only). Native `sys` surface addition remains open for the sibling lane.

## Finding W110-5 (NEXT BLOCKER, sibling-owned): v0 coroutine stubs never execute

`async def` functions produce inert marker objects (interp.rs:5993 —
wave-96 v0 stub: Namespace obj with `__ap_coroutine_send__` that raises
`StopIteration('')`). Consequences, all reproduced with minimal probes:

1. Coroutine bodies NEVER run — `async def h(): return 7; h().send(None)`
   → `StopIteration('')` (CPython: body runs, `StopIteration(7)`).
2. `await X` never binds X's value — awaiting chains complete instantly
   with `''`.
3. Every asyncio Task "completes" immediately with result `''`
   (`Task.__step` → `coro.send(None)` → StopIteration('') → set_result('')).
4. `async def` WITH yield returns a **sync `generator`** object (interp
   routes `is_generator` before `is_async`), missing
   `asend/__aiter__/__anext__` — `async for` over it cannot run
   (`AttributeError` path); CPython: `async_generator`.
5. The crawl's `open_spider_async → await scraper.spidermw.process_start()`
   chain (async-gen consumption) therefore terminates with the spider never
   opened: `join enter active=1` → DL fires → reactor exits, no PARSE calls.

**Required fix** (generators.rs/interp.rs, NOT this lane): route
`def.is_async && !def.is_generator` through the real fiber machinery
(start the frame like a generator; completion maps to `StopIteration(value)`
with the true return value; `send()` resumes; `__await__` returns self).
Async-gen objects (`is_async && is_generator`) need the async-gen protocol
surface (asend/athrow/aclose/__aiter__/__anext__).

## Finding W110-6: zope Interface subclass binding (fixed this lane)

`reactor.installResolver → assert IResolverSimple.providedBy(resolver)` —
`class IFoo(Interface)` via `__mro_entries__` yields a base of
InterfaceClass; `IFoo.providedBy` resolved to the RAW function (unbound) →
`TypeError: providedBy() missing 1 required positional argument: 'obj'`.
**Fixed** in `vendored_stdlib/zope/interface/_shim.py`: the
`__mro_entries__` hook now returns `_InterfaceSubclass` (metaclass
pre-binds `providedBy`/`implementedBy`/`names`/… onto each subclass with
`cls` closed over). Verified identical behavior on CPython with the same
shim.

## Chain state after wave-110

`W95 spider created → W96 settings → W97 exts+middlewares → W95 engine
created → W95 pre-open_spider_async → ensure_future → Task __step →
reactor run_forever spinning → resolver installed (zope fixed) →
crawl Deferred fires (stub-coroutine instant completion) → exit 0, no
items`. Next wave: implement W110-5 (real coroutine frames) in the
generators/interp lane; then the crawl should reach `Spider opened` /
first PARSE.

## Files changed (this lane)

- `vendored_stdlib/cryptography/**` — NEW (24 files, v0 shim)
- `vendored_stdlib/zope/interface/_shim.py` — Interface subclass binding
- `vendored_stdlib/asyncio/tasks.py` — _TaskNameCounter (itertools.count
  `__next__` workaround; W-EF/W-EF2/W-T probes from prior waves still in
  tree, env-safe)
- `vendored_stdlib/asyncio/base_events.py` — asyncgen hooks no-op
- `crates/ap-run/src/builtins_modules.rs` — time.gmtime/localtime accept
  float epochs

## Probes (all in /tmp, none staged into site-packages)

- probe_crypto_surface.py / crypto_surface.json (CPython surface pin)
- probe_crypto_shim.py (14 CPython checks), probe_crypto_ap.py (8 ap checks)
- probe_telnet_chain.py (conch.telnet/insults/telnet-ext/scrapy all OK)
- probe_time.py, probe_count.py, probe_coro{,2-9}.py, probe_gen.py,
  probe_agen{2-15}.py, probe_loop.py, probe_zope{,2-6}.py,
  probe_next_blocker.py

## Gates

- `cargo build -p ap-cli --bin ap-run-eval` — 0 errors
- `cargo test -p ap-parse --test corpus` — 7 passed / 0 failed
- `cargo test -p ap-cli --test stdlib_modules_differential` — 48/0 (twice)
- ap-run differentials spot-checked green: method_surface 34/0,
  classes 29/0, import 2/0/1i, fix_pass 6/0, name_resolution 24/0
- SCRAPY IMPORT OK 2.19.0 — verified in-chain (probe_telnet_chain.py)
- NOTE: `cargo test -p ap-run` (full) timed out at 420s on this machine —
  the named differential suites above were run individually instead;
  recommend orchestrator run the full workspace gate on a clean tree.
