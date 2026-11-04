# Leaf A-3: interpreter exception-flow fixes from the RustPython campaign

## DISPATCH HISTORY
- First dispatch (sa-0-bdc24419) DIED of model degeneration (repetition
  loop) 33 min in. Salvage verified by orchestrator probes: os.open
  arm-miss FIXED (now FileNotFoundError instead of hard-exit), os.utime
  tuple-length TypeError LANDED; except-Exception escape, OSError str,
  Opaque naming/stat_result sequence still broken; AT_FDCWD constant
  edits lost with the loop.
- CONTINUATION dispatched (sa-0-c87f7258) with surviving-state brief.

## Source
Findings F1/F2 verified firsthand 2026-10-25 (plans/antipythony/
05-ecosystem-t2/PROGRESS.md). These are NOT donor ports - they are
interpreter bugs the campaign's differential work exposed.

## UNBLOCK CONDITION
Dispatch only after A-2 lands (crates/ap-run/src/builtins_modules.rs and
the builtins paths in interp.rs are A-2-owned until then).

## Findings + fixes (repro-verified)
1. F1 SILENT HARD-EXIT on unimplemented call_os arms.
   Repro: `import os; print("before"); fd = os.open("/tmp/x", os.O_RDONLY)`
   -> prints "before", exits rc=0, stdout TRUNCATED, no traceback, no
   exception. Same for os.symlink. Registered-os-names with NO call_os arm
   (wave-1 sweep): chmod, open, readlink, removedirs, scandir, statvfs,
   symlink, umask, walk.
   Fix: the arm-miss fall-through path must raise AttributeError (matching
   how chown/getpgrp already behave) - find why the process dies silently
   (likely an unwrap/exit in the marker path) and make the miss raise.
2. F2 UNCAPTCHAABLE native exceptions.
   Repro: `import binascii; binascii.unhexlify(b"abc")` inside
   try/except Exception -> Error raised but the except block is SKIPPED
   (traceback prints, rc=1). Manual `raise binascii.Error("x")` IS
   catchable; issubclass(binascii.Error, Exception) is True.
   Fix: native-raised exception objects must carry the same class identity
   the raise path uses. Scope: wherever call_native builds the Error value
   (builtins_modules marker path) vs where try/except matches class
   identity (interp.rs). The interp.rs region is THIS leaf's to touch.
3. OSError taxonomy + str format (same probe family, fix together).
   - os.kill(1,0) raises OSError(1); CPython raises PermissionError(1).
     EPERM->PermissionError, ESRCH->ProcessLookupError (kill already does
     ESRCH per wave 1 - reuse that mapping for all call_os errors).
   - str(OSError) renders "(1, 'Operation not permitted')" in
     exc_hier.rs py_str; CPython: "[Errno 1] Operation not permitted".
     Add an OSError special case to py_str (exc_hier.rs is interp-owned).
4. os.utime tuple-length validation: times=(1,2,3) accepted; CPython
   raises TypeError "'times' must be either a tuple of two ints or None"
   (pinned 2026-10-25). Likely one arity check in the utime arm.

## Gates
- Repros 1-4 now behave byte-exact vs python3.14 (probe each).
- New differential cases in stdlib_modules_differential.rs: os.open miss
  raises AttributeError; unhexlify odd-length caught by except Exception;
  PermissionError on os.kill(1,0); utime 3-tuple TypeError.
- cargo workspace 0 failed; pytest >= 1951; corpus non-decreasing.

## Ownership
interp.rs (exception paths only), exc_hier.rs, builtins_modules.rs
(dispatch fall-through + utime arm). Confirm A-2 agent is finished before
starting.
