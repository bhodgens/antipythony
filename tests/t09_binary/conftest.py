# T-09 session hygiene: reap leaked t09_* temp dirs.
#
# Why this exists: the suite historically used bare tempfile.mkdtemp with no
# cleanup registration, so killed runs leaked 128 dirs / ~43.8 GB into
# $TMPDIR between Nov 11 and Nov 26 2026. The test module now tracks every
# mkdtemp and removes it via atexit; this autouse fixture is the backstop for
# dirs leaked before that fix (or by SIGKILL, which skips atexit).
#
# The t09_* prefixes are project-unique, so globbing them in the session
# tempdir is safe.

import shutil
import tempfile
from pathlib import Path

import pytest

_T09_GLOB_PREFIXES = ("t09_build_", "t09_repro_", "t09_cwd1_", "t09_cwd2_")


def _reap_stale_t09_dirs() -> None:
    tmp = Path(tempfile.gettempdir())
    for prefix in _T09_GLOB_PREFIXES:
        for d in tmp.glob(prefix + "*"):
            if d.is_dir():
                shutil.rmtree(d, ignore_errors=True)


@pytest.fixture(scope="session", autouse=True)
def _t09_temp_hygiene():
    _reap_stale_t09_dirs()
    yield
    _reap_stale_t09_dirs()
