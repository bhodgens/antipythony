"""Unified CPython 3.14 reference-interpreter discovery for the pytest suites
(robustness-ci leaf 01.01; closes the pytest-side gap 00.02 left open).

Single source of truth mirroring the Rust helper at
``crates/ap-run/tests/common/mod.rs`` and the logic at
``crates/ap-conform/src/lib.rs`` (master.md "Reference python discovery" +
"Skip policy" contracts). tests/README.md rule 5: anything that runs
"CPython" must require a real Python 3.14 via a ``--version`` string check —
never a bare ``python3`` fallback.

Resolution ladder (a candidate is accepted ONLY if its ``--version`` output
reports Python 3.14, checked on BOTH stdout and stderr — pre-3.8 interpreters
print to stderr):

1. ``AP_CONFORM_REFERENCE_PYTHON`` — explicit override. If set but the
   version check fails, warn and fall through (same as the Rust helper).
2. PATH ``python3.14`` (``shutil.which``).
3. ``/opt/homebrew/bin/python3.14`` — host convenience fallback, used ONLY
   when the skip-to-fail policy is OFF. CI (``AP_REQUIRE_REFERENCE=1``) is
   strict: env -> PATH only, no hardcoded host paths, exactly like the Rust
   helper which has no Homebrew fallback at all.

Skip policy (master.md "Skip policy" contract):
- ``AP_REQUIRE_REFERENCE=1`` and nothing resolves -> ``pytest.fail`` with the
  canonical message ``"reference interpreter not found (AP_REQUIRE_REFERENCE=1)"``
  — a runner without python3.14 must go red, never report green (finding #3).
- otherwise -> ``pytest.skip`` (visible skip; a local box lacking the
  reference stays honest instead of crashing).

Usage in a suite::

    import sys
    from pathlib import Path
    REPO_ROOT = Path(__file__).resolve().parents[2]
    sys.path.insert(0, str(REPO_ROOT / "tests" / "tools"))
    from _ref_python import reference_or_skip

    def test_something():
        ref = reference_or_skip()          # str; raises skip/fail when absent
        subprocess.run([ref, "case.py"], ...)

Call it INSIDE test bodies (not at module import) so suites keep collecting
and running their non-differential tests on hosts without the reference.
Resolution is cached per process (one cheap ``--version`` spawn each).
"""

from __future__ import annotations

import functools
import os
import shutil
import subprocess
import sys
from pathlib import Path

# NOTE: `pytest` is imported lazily (inside reference_or_skip) so plain
# generator scripts (tests/t07_gc/_gen_expected.py, tools) can import
# discover_reference without pytest on the path.

CANONICAL_FAIL_MSG = "reference interpreter not found (AP_REQUIRE_REFERENCE=1)"
_HOST_FALLBACK = "/opt/homebrew/bin/python3.14"


def _is_python_314(program: str) -> bool:
    """True when ``program --version`` succeeds and reports Python 3.14.

    Pre-3.8 interpreters print --version to stderr, so check both streams
    (same discipline as ap-conform's ``is_python_314``).
    """
    try:
        out = subprocess.run([program, "--version"], capture_output=True,
                             timeout=10)
    except (OSError, subprocess.SubprocessError):
        return False
    if out.returncode != 0:
        return False
    text = (out.stdout + out.stderr).decode(errors="replace")
    return "Python 3.14" in text


def require_reference() -> bool:
    """True when the skip-to-fail policy is requested (CI sets it; master.md
    "Skip policy"). Any value other than empty/"0" counts — mirrors the Rust
    helper."""
    v = os.environ.get("AP_REQUIRE_REFERENCE", "")
    return bool(v) and v != "0"


@functools.lru_cache(maxsize=1)
def discover_reference() -> "str | None":
    """Run the resolution ladder once; return the verified program or None."""
    env = os.environ.get("AP_CONFORM_REFERENCE_PYTHON", "").strip()
    if env:
        if _is_python_314(env):
            return env
        print(f"warning: AP_CONFORM_REFERENCE_PYTHON={env!r} does not report "
              "Python 3.14; falling back to PATH python3.14",
              file=sys.stderr)
    on_path = shutil.which("python3.14")
    if on_path and _is_python_314(on_path):
        return on_path
    if not require_reference() and Path(_HOST_FALLBACK).is_file() \
            and _is_python_314(_HOST_FALLBACK):
        # Host convenience only in non-strict mode (keeps today's local
        # behavior byte-for-byte on the pinned dev box); CI is strict.
        return _HOST_FALLBACK
    return None


def reference_or_skip() -> str:
    """Verified reference interpreter, or skip/fail per the skip policy.

    Raises ``pytest.fail`` (canonical message) when AP_REQUIRE_REFERENCE=1 and
    nothing resolves; raises ``pytest.skip`` otherwise. Mirrors the Rust
    ``reference_or_skip()`` semantics.
    """
    found = discover_reference()
    if found is not None:
        return found
    import pytest
    if require_reference():
        pytest.fail(CANONICAL_FAIL_MSG, pytrace=False)
    pytest.skip("python3.14 not found (set AP_CONFORM_REFERENCE_PYTHON or "
                "put python3.14 on PATH)")


if __name__ == "__main__":  # tiny manual probe: python _ref_python.py
    r = discover_reference()
    print(f"require={require_reference()} resolved={r}")
    sys.exit(0 if r else 1)
