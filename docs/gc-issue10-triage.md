# Issue #10 triage — GC corpus regressions (gc009 / gc013 / gc015)

Wave-109 fix-triage (triage-only lane; runtime fixes owned elsewhere).
Baseline: repo `main` @ 98b2964, corpus 233/271, gc family 15/17 (2 hard
fails). Reference: `/opt/homebrew/bin/python3.14` (3.14.7; contract pins
3.14.0 — the recorded-metas caveat per docs/conformance.md and the
golden-provenance metas).

Prior audits: `.hermes/audits/2026-10-30-bughunt-wave.md` + d136693
addendum refuted the "GC root-scan over-sweep" claim (subtract-refs
invariant holds). This triage confirms that refutation independently and
narrows the surviving wave-109 suspects:

- **guard-nesting**: NOT implicated by any of the three failures
  (single-level collections only in gc009/gc013/gc015; no nested
  `gc.collect()` from inside a finalizer). The known `CollectGuard`
  inner-drop footgun (weakbridge.rs unit test
  `nested_guards_clear_the_flag_at_inner_drop`) remains real MMTk-swap
  debt but is not this issue's cause.
- **lowering `expect()`**: not exercised here — no panic path triggers;
  all three cases exit 0.

## Case-by-case

### gc009_finalizer_raises_unraisable — FAIL (bytes, rc 0 both)

**Divergence (bytes-exact, first diff at stdout offset 198):**

- ref:  `source echo: '    raise ValueError("shape-check")'` (4-space
  indent — CPython strips the source line's leading whitespace and
  re-indents 4)
- ours: `source echo: '            raise ValueError("shape-check")'`
  (4-space pad + the RAW source line incl. its 8-space file indent)

Minimal repro (`/tmp/gc009_min.py`, 2-level nesting shows it too; deeper
nesting scales the delta):

```python
import io, sys, gc
buf = io.StringIO()
class Bad:
    def __del__(self):
        raise ValueError("shape-check")
sys.stderr = buf
b = Bad(); del b; gc.collect()
sys.stderr = sys.__stderr__   # (corpus case swaps back to saved)
print(repr(buf.getvalue()))
```

ref echoes `    raise ValueError("shape-check")`; ours echoes the line
with its full file indentation preserved.

**Root cause (high confidence, file:line):**
`crates/ap-run/src/gcsupport.rs:424-427` — `report_unraisable` emits
`push_str("    "); push_str(src.trim_end())`. It strips trailing
whitespace but never strips LEADING whitespace. Commit 98b2964 fixed the
regular traceback renderer the same day (`interp.rs`: renderer now uses
`src_line.trim()` via `source_line_for`) but the sibling unraisable
renderer in gcsupport.rs was missed — an asymmetry between the two
renderers, not a GC-semantics bug. **Classification: wrong-renderer
(Class-X text shaping), not wrong-finalizer-semantics.** The unraisable
hook path (structured args, exc_type/exc_value/exc_traceback, rc
preservation) is byte- and behavior-correct: every other pinned line of
gc009 matches the reference exactly.

**Owning file:** `crates/ap-run/src/gcsupport.rs` (`report_unraisable`,
source-echo push).

**Fix sketch (words, not applied):** mirror the 98b2964 renderer fix —

```rust
// today:
text.push_str("    ");
text.push_str(src.trim_end());
// fix:
text.push_str("    ");
text.push_str(src.trim());          // strip leading + trailing, like
                                    // the interp.rs traceback renderer
```

Optionally factor the echo into the same `source_line_for` helper so the
two renderers cannot drift again.

### gc013_get_objects_filtered — PASSES at 98b2964 (brief's claim stale)

Re-run firsthand: `ap-conform run --corpus tests/corpus/gc/gc013... --impl
target/debug/ap-run-eval` → PASS; also PASS via `antipythony run` impl and
byte-identical vs live python3.14 (`held count: 4 / after drop+collect: 0
/ cycle survivor count: 0`, rc 0). The registry retain + subtract-refs
invariant holds for the filtered-get_objects projection. **If issue #10
still lists gc013 as failing, the issue text is stale** — or the failure
requires a state (tier seam, threading interleaving) not present at this
commit; nothing to fix on this tree. A passing regression fence was
authored anyway (gc020, see below).

### gc015_weakref_finalize_api — FAIL (bytes, rc 0 both)

**Divergence (bytes-exact, first diff at stdout offset 49):**

```
line                                         ref          ours
after collect: ['bye'] alive:   ...         False        True
detach fields: 4 True ...      ...         False        True
```

The finalize function FIRES at the right time (calls == ['bye'] after
`gc.collect()`), reinvoke returns None, detach returns the 4-tuple,
post-fire detach returns None — exactly-once semantics are all correct.
ONLY the `fin.alive` reads are wrong: **alive is invariantly True** in
our runtime, from registration through fire, detach, and reinvoke.

Minimal repro:

```python
import weakref, gc
class F: pass
calls = []
obj = F()
fin = weakref.finalize(obj, calls.append, "bye")
del obj; gc.collect()
print(calls, fin.alive)   # ref: ['bye'] False ; ours: ['bye'] True
```

**Root cause (high confidence, file:line, proven by read-path audit +
behavioral corroboration):**

1. `crates/ap-run/src/gcsupport.rs:768` — `call_weakref_finalize` seeds
   the token object with a literal `alive = Bool(true)` attribute at
   creation.
2. `crates/ap-run/src/interp.rs:9338-9350` — `attr_get` early-returns ANY
   explicitly stored attribute on `ObjKind::Opaque` objects BEFORE the
   per-type native surfaces are consulted (the hashlib escape hatch only
   exempts `hexdigest`/`digest`).
3. Therefore `fin.alive` reads the frozen creation attr forever; the
   correct bookkeeping arm — `builtins_modules.rs:13838-13839`,
   `"_finalize_token" => "alive" => finalize_alive(o)` — is DEAD CODE for
   attribute reads.

Corroboration (the state machine under the dead attr IS correct):
`finalize_call` marks the entry detached on fire (post-fire `fin()`
returns None; post-fire `detach()` returns None), and `finalize_detach`
sets the same flag — yet `alive` never flips. Only a read-shadowing bug
explains a state flag that mutates but never observes. Additional
evidence: `type(fin)` reports `<class 'object'>` (cosmetic; CPython
reports `<class 'weakref.finalize'>`) — the token is a bare Opaque, so
instance-dict precedence over the property surface is exactly the
shadowing path above.

**Classification: missing-API-surface read path (Class-S: wrong values),
not GC sweep overreach and not finalize-timing.**

**Owning file:** `crates/ap-run/src/interp.rs` (`attr_get` Opaque
early-return) + `crates/ap-run/src/gcsupport.rs` (drop the stale
`alive` attr seed at token creation).

**Fix sketch (either half suffices; both is belt-and-braces):**

- Remove the `tattrs.set(s("alive"), Value::Bool(true))` seed in
  `call_weakref_finalize` (gcsupport.rs ~768) so the stored attr can't
  shadow; OR
- Extend the `attr_get` Opaque exemption list (interp.rs ~9340) to skip
  the early-return for tokens named `_finalize_token`/`_weakref_token`
  (mirroring the `hexdigest`/`digest` escape), letting
  `builtins_modules.rs` native surface answer `alive` dynamically via
  `finalize_alive`.

Watch-out for the fixer: the same shadowing likely affects any OTHER
dynamic property seeded as a literal attr on Opaque tokens (audit
`_weakref_token` surfaces in the same pass).

## Fence (authored this lane)

- `tests/corpus/gc/gc018_unraisable_source_echo_strip.py` — gc009 shape
  isolated: 3-level nested `__del__`, masked address + File line, pins
  the 4-space stripped source echo. xfail on current binary (raw-indent
  echo); flips when gcsupport.rs fix lands.
- `tests/corpus/gc/gc019_finalize_alive_bookkeeping.py` — gc015 shape
  isolated: alive-before / alive-after-collect / reinvoke / detach
  fields / detached-no-callback. xfail on current binary (alive
  invariantly True); flips when the interp/gcsupport fix lands.
- `tests/corpus/gc/gc020_get_objects_count_fence.py` — gc013 shape
  regression fence: PASSING today by design (gc013 passes); guards the
  registry-count invariant the stale issue entry claimed broken.

Pins generated via `tests/t07_gc/_gen_expected.py` (3x determinism, all
cases non-flapping) and provenance metas re-recorded (date 2026-10-31,
generator sha unchanged: 288ce618…). Existing 17 cases' `.out/.err/.rc`
byte-identical after regeneration (git diff: metas only).

## Gate results (this tree, post-fence)

- `ap-conform` gc family vs `ap-run-eval`: 16/20, hard fails exactly
  {gc009, gc015, gc018, gc019}; gc020 passes.
- `pytest tests/t07_gc/ -q`: 131 passed, 23 xfailed, 0 failed (the 3 new
  cases fail ONLY via the pre-existing strict=False ap-conform xfail
  markers; no hard reds, no unexpected xpass).
- `pytest tests/corpus_traceability/ -q`: 484 passed, 1 skipped
  (citations + golden provenance accept the new pins/metas).
- `cargo test -p ap-gc`: 14 + 9 + 0 passed, 0 failed.

## Wave-109 hypothesis disposition

- root-scan over-sweep: REFUTED (d136693; independently corroborated —
  gc013/gc020 pass, counts exact).
- guard-nesting: NOT the cause of any of the 3 cases (no nested
  collection in any repro); remains queued MMTk debt, unrelated.
- lowering expect(): not exercised; no panic in any repro.
- ACTUAL causes: (a) unraisable renderer indent-strip gap (gcsupport.rs,
  Class-X text); (b) Opaque attr-read shadowing of finalize.alive
  (interp.rs + gcsupport.rs, Class-S). Both are renderer/attr-surface
  bugs, NOT collector semantics.
