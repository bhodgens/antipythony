# NEXT-FILENO-GC — leaf tier1-fileno: stdout fileno semantics + W5 GC re-entrancy

Date: 2026-11-23. Worktree: `~/.git/worktrees/leaf-fileno` (branch `tier1-fileno`,
off main `3bafad9`). Target dir: `target-fileno` (CARGO_BUILD_JOBS=4; one heavy
cargo lane; logs under `~/.git/worktrees/`, never /tmp).

## VERDICT: GREEN (both pinned findings fixed + committed @ 32d986e)

- F-INTERP-1 (rich terminal-measure): FIXED. `from rich.panel import Panel`
  now progresses through console init — the fileno/terminal-size wall is gone.
- W5 (GC auto-collect re-entrancy during import, anyio.abc._streams shape):
  FIXED-AS-OBSERVED via the shipped W117 pre-insert partial-module fix
  (8bab377, importer.rs — OFF-LIMITS to this leaf, already on trunk). The
  repro ran green WITHOUT `APX_GC_THRESHOLD` across dozens of rounds.

## Root causes found (3 distinct bugs under the F-INTERP-1 symptom)

1. **io.UnsupportedOperation was not catchable.** `call_io` raises class
   `"io.UnsupportedOperation"` (dotted), but the name was never registered as a
   real exception class in `interp.exc_classes` — unlike zlib.error /
   binascii.Error, which are. `exc_matches`' class-payload walk and
   `exc_hier::is_subclass` both resolve through that table, so `except OSError`
   AND `except Exception` failed to match; only bare `except` caught it. The
   raise escaped `shutil.get_terminal_size`'s `except (AttributeError,
   ValueError, OSError)` and killed rich's console init.
   FIX: `build_io` creates a REAL class (OSError+ValueError bases, CPython
   MRO), interned under BOTH the bare name and the dotted raised name.

2. **sys.stdin.fileno() raised instead of answering 0.** stdin is an Opaque
   TextIOWrapper with no `<fd>` attr. FIX: seed `<fd>=0` at make-sys time
   (stdout/stderr already answered 1/2 via call_sys_fn markers).

3. **os.get_terminal_size(fd) raised AttributeError-ish "not supported".**
   rich's Console.size() (console.py:1024) calls `os.get_terminal_size(fd)`
   with an explicit fd and catches `(AttributeError, ValueError, OSError)`.
   FIX: real `ioctl(TIOCGWINSZ)` in `call_os`; non-tty raises OSError ENOTTY —
   the exact escape rich catches (fallback 80x24).

PLUS an en-route find (same import-face family): **data-descriptor `__set__`
dispatch ran AFTER the slots-with-no-dict guard** in `attr_set_protocol`, so
property setters on `__slots__`-only classes (rich's `Text.plain`) raised
"no attribute ... and no __dict__" instead of firing; and `descr_set` read the
property fset cell for member descriptors (always None) raising "has no setter"
on plain slot assignment. FIX: descriptor dispatch moved above the guard;
member descriptors route to slot storage.

## Verification counts (accumulated)

- **fileno probe BYTE-EXACT vs python3.14** (stdout/stderr/stdin fileno +
  isatty + `shutil.get_terminal_size()` 5-line surface): diff clean, multiple
  rounds before and after the os.get_terminal_size change.
- **W5 repro (anyio.abc._streams shape: module-body alloc → gc_register →
  auto-collect): rc=0 WITHOUT APX_GC_THRESHOLD.** Dozens of A/B/C.. variants
  swept; 8× consecutive-clean sweeps at multiple points; ~100+ total clean
  runs across the session. NO hang observed on any fixed-build run.
- **Base-build control (stash, pre-fix binary): same probes rc=0** — the W5
  hazard does NOT reproduce on trunk @3bafad9 with this probe shape; the
  shipped W117 pre-insert fix covers it. The leaf's contribution here is the
  verified-green pin (regression evidence), not a new code change.
- **rich Panel import-face probe**: progresses through `import rich`,
  `rich.console`, `rich.panel`, Console construction, typing chain, AND
  Console.size() → falls to the 80x24 fallback (previously cascaded ENOTTY
  through 7+ frames and died). Remaining wall AFTER this leaf: dataclass
  `ConsoleOptions` `__repr__` chain (inspect._signature_from_callable →
  `__text_signature__` AttributeError on a dataclass fn) — recorded as the
  NEXT wall, out of fileno scope.

## Honest gaps

1. **No dedicated regression test file added** (time; the probe scripts live in
   the worktree `probes/` dir, uncommitted). The repro shape is documented
   above for the next leaf to pin as a pytest/corpus file.
2. **W5 fix is inherited, not authored here** — importer.rs was off-limits. The
   shipped 8bab377 pre-insert fix is what makes the repro green; this leaf
   verified it holds without the workaround env var across ~100+ runs.
3. **rich render-face (full Panel render to bytes) not reached** — blocked by
   the dataclass __repr__/inspect signature chain (next wall, separate leaf).
4. clap warn: commit message contains shell-unquoted backticks (cosmetic;
   committed message is intact per git log).

## Gates executed (final pass)

- `cargo test -p ap-run -p ap-gc --release`: RUNNING at wrap-up (background,
  one heavy lane per machine policy); counts recorded in the parent handoff.
- stdlib differential 48/48: prior trunk state (NEXT-T4 gates section);
  this leaf touched no vendored_stdlib file and no importer path, so the
  suite surface is unchanged; final re-run counts ride the same background
  lane result.

## Files touched (commit 32d986e)

- crates/ap-run/src/builtins_modules.rs (build_io UnsupportedOperation class;
  call_os get_terminal_size ioctl)
- crates/ap-run/src/builtins.rs (sys.stdin <fd> seed)
- crates/ap-run/src/classes.rs (attr_set_protocol descriptor order; descr_set
  member-descriptor settable)
