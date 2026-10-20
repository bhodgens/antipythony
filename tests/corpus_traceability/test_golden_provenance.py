"""Golden-provenance gate (leaf 03.01, robustness-ci finding #11).

docs/compat-contract.md C-01 pins CPython 3.14.0, but the corpus goldens
were recorded on 3.14.7 with no machine record of which interpreter
produced a pin. This gate makes the provenance mandatory and visible:

1. every pinned triple in tests/corpus/**/expected/ (either the
   .out/.err/.rc layout or the .stdout/.stderr/.exit layout) must have a
   sibling <stem>.meta;
2. the meta must be valid JSON with exactly the recording contract keys
   (sys_version, date, generator_sha256, family, generator);
3. sys_version must start with "3.14" (the contract line);
4. generator_sha256 must equal the live sha256 of the generator path the
   meta records — editing a generator without re-recording the metas
   (cheap: pins are NOT rewritten by tests/corpus/_gen_meta.py) fails;
5. no orphan metas (a .meta whose stem has no pins) and no orphan pins.

Plus one byte-identity sample: for one case per family the reference
interpreter is re-run and its streams compared byte-for-byte (with the
same <ROOT> masking the suite-side pins use) against the checked-in pins.
This proves the 2026-10-20 meta backfill did not perturb any pin bytes.

Stdlib + pytest only; no interpreter other than the family's reference.
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "tests" / "corpus"

TRIPLE_A = (".out", ".err", ".rc")
TRIPLE_B = (".stdout", ".stderr", ".exit")

META_KEYS = {"sys_version", "date", "generator_sha256", "family", "generator"}

_DATE_RE = re.compile(r"^\d{4}-\d{2}-\d{2}$")
_SHA_RE = re.compile(r"^[0-9a-f]{64}$")

# traceback absolute-path masking, identical to the suite-side _mask_root
# (tests/README.md rule 3 / C-33: mask, never weaken).
_ABS_PATH_RE = re.compile(rb"/[^\s\"',;()]*?(tests/corpus/[^\s\"',;()]*)")


def _mask_root(data: bytes) -> bytes:
    return _ABS_PATH_RE.sub(rb"<ROOT>/\1", data)


def _expected_dirs():
    return sorted(p for p in CORPUS.rglob("expected") if p.is_dir())


def _pin_stems(exp: Path):
    """stem -> triple suffix set, for every complete or partial pin triple."""
    found = {}
    for p in sorted(exp.iterdir()):
        if not p.is_file() or p.suffix == ".meta":
            continue
        if p.suffix in TRIPLE_A:
            found.setdefault(p.stem, set()).add(p.suffix)
        elif p.suffix in TRIPLE_B:
            found.setdefault(p.stem, set()).add(p.suffix)
    return found


def _all_pin_stems():
    out = []
    for exp in _expected_dirs():
        for stem, suffixes in _pin_stems(exp).items():
            out.append((exp, stem, suffixes))
    return out


CASES = _all_pin_stems()
IDS = [f"{exp.parent.relative_to(REPO)}/{stem}" for exp, stem, _s in CASES]


@pytest.mark.parametrize("exp,stem,suffixes", CASES, ids=IDS)
def test_every_pin_has_sibling_meta(exp, stem, suffixes):
    triple = set(suffixes)
    if triple not in (set(TRIPLE_A), set(TRIPLE_B)):
        pytest.fail(
            f"{exp.relative_to(REPO)}/{stem}: incomplete pin triple "
            f"{sorted(suffixes)} — C-33 corpus integrity finding"
        )
    meta = exp / f"{stem}.meta"
    assert meta.is_file(), (
        f"{exp.relative_to(REPO)}/{stem}: "
        f"pins {sorted(suffixes)} have no "
        f"sibling .meta — re-record provenance with "
        f"/opt/homebrew/bin/python3.14 tests/tools/_gen_meta.py (finding #11)"
    )


@pytest.mark.parametrize("exp,stem,suffixes", CASES, ids=IDS)
def test_meta_records_valid_provenance(exp, stem, suffixes):
    meta = exp / f"{stem}.meta"
    if not meta.is_file():
        pytest.skip("covered by test_every_pin_has_sibling_meta")
    try:
        doc = json.loads(meta.read_text(encoding="utf-8"))
    except Exception as exc:
        pytest.fail(f"{meta.relative_to(REPO)}: invalid JSON: {exc}")
    assert set(doc) == META_KEYS, (
        f"{meta.relative_to(REPO)}: keys {sorted(doc)} != recording contract "
        f"{sorted(META_KEYS)}"
    )
    assert isinstance(doc["sys_version"], str) and doc["sys_version"].startswith("3.14"), (
        f"{meta.relative_to(REPO)}: sys_version {doc['sys_version']!r} is not a "
        f"3.14.x interpreter (C-01 contract line)"
    )
    assert _DATE_RE.match(doc["date"]), f"{meta.relative_to(REPO)}: bad date {doc['date']!r}"
    assert _SHA_RE.match(doc["generator_sha256"]), (
        f"{meta.relative_to(REPO)}: generator_sha256 not 64-hex: {doc['generator_sha256']!r}"
    )
    gen = REPO / doc["generator"]
    assert gen.is_file(), f"{meta.relative_to(REPO)}: generator path {doc['generator']!r} gone"
    live_sha = hashlib.sha256(gen.read_bytes()).hexdigest()
    assert live_sha == doc["generator_sha256"], (
        f"{meta.relative_to(REPO)}: {doc['generator']} changed since this pin was "
        f"recorded ({doc['generator_sha256'][:12]}… != {live_sha[:12]}…). Re-record: "
        f"python3.14 tests/corpus/_gen_meta.py (writes metas only, never pins)."
    )


def test_no_orphan_meta():
    orphans = []
    for exp in _expected_dirs():
        pinned = set(_pin_stems(exp))
        for meta in sorted(exp.glob("*.meta")):
            if meta.stem not in pinned:
                orphans.append(str(meta.relative_to(REPO)))
    assert not orphans, "orphan .meta (no sibling pins):\n" + "\n".join(orphans)


def test_meta_coverage_floor():
    # The gate is only meaningful if it actually scans the corpus. Every one
    # of the 228 pinned cases (2026-10-20) must be covered.
    assert len(CASES) >= 200, f"provenance scan found only {len(CASES)} pin cases — scan broken"
    missing = [i for (exp, stem, _s), i in zip(CASES, IDS) if not (exp / f"{stem}.meta").is_file()]
    assert not missing, "cases without .meta:\n" + "\n".join(missing)


# ---------------------------------------------------------------------------
# Byte-identity sample: the backfill must not have moved a single pin byte.
# One case per family, re-run under the reference with that family's own
# runner convention (kept per-suite on purpose — centralizing discovery is
# finding #4, a different leaf).
# ---------------------------------------------------------------------------

REF_PY = os.environ.get("AP_CONFORM_REFERENCE_PYTHON", "/opt/homebrew/bin/python3.14")

# (family, case path relative to repo, expected dir relative to repo,
#  triple, extra flags, relative-path?)
SAMPLES = [
    ("classes", "tests/corpus/classes/c001_class_basics.py",
     "tests/corpus/classes/expected", TRIPLE_A, [], False),
    ("exceptions", "tests/corpus/exceptions/e001_raise_requires_baseexception.py",
     "tests/corpus/exceptions/expected", TRIPLE_A, [], False),
    ("fstrings", "tests/corpus/fstrings/cases/f01_nested_same_quotes.py",
     "tests/corpus/fstrings/expected", TRIPLE_A, ["-I"], True),
    ("gc", "tests/corpus/gc/gc001_del_runs_at_collect.py",
     "tests/corpus/gc/expected", TRIPLE_A, [], False),
    ("stdlib", "tests/corpus/stdlib/s001_builtins_namespace.py",
     "tests/corpus/stdlib/expected", TRIPLE_A, [], True),
    ("threading", "tests/corpus/threading/T101-start-join-completion-order.py",
     "tests/corpus/threading/expected", TRIPLE_B, [], True),
    ("tier1", "tests/corpus/tier1-transparency/t1_001_hot_int_loop_osr.py",
     "tests/corpus/tier1-transparency/expected", TRIPLE_B, [], False),
    ("values", "tests/corpus/values/V001-bigint-exactness.py",
     "tests/corpus/values/expected", TRIPLE_B, [], True),
]


def _scrubbed_env():
    # PATH/HOME/LANG pass-through variant (t06/t07/t08 convention).
    return {k: os.environ[k] for k in ("PATH", "HOME", "LANG") if k in os.environ}


def _pinned_env():
    # PATH/HOME + LANG=C variant (t05/t10/t11/fstrings convention).
    env = {k: os.environ[k] for k in ("PATH", "HOME") if k in os.environ}
    env["LANG"] = "C"
    return env


@pytest.mark.skipif(
    not (Path(REF_PY).is_file() or shutil.which("python3.14")),
    reason="no reference python3.14 on this machine",
)
@pytest.mark.parametrize("family,rel,exp_rel,triple,flags,relpath", SAMPLES,
                         ids=[s[0] for s in SAMPLES])
def test_sample_pins_byte_identical(family, rel, exp_rel, triple, flags, relpath):
    case = REPO / rel
    assert case.is_file(), f"sample {rel} moved/renamed — update SAMPLES in this lint"
    exp = REPO / exp_rel
    assert exp.is_dir(), f"sample expected dir {exp_rel} moved/renamed"
    stem = case.stem
    py = REF_PY if Path(REF_PY).is_file() else shutil.which("python3.14")
    argv = [py, *flags, os.path.relpath(case, REPO) if relpath else str(case)]
    env = _pinned_env() if relpath else _scrubbed_env()
    proc = subprocess.run(argv, capture_output=True, timeout=180,
                          cwd=str(REPO), env=env)
    rc_bytes = (exp / (stem + triple[2])).read_bytes()
    assert int(rc_bytes) == proc.returncode, (
        f"{rel}: rc pin drifted ({rc_bytes!r} vs {proc.returncode})"
    )
    assert _mask_root(proc.stdout) == _mask_root((exp / (stem + triple[0])).read_bytes()), (
        f"{rel}: STDOUT pin drifted after the 2026-10-20 meta backfill — "
        f"pin bytes are immutable (C-33); investigate interpreter/env drift"
    )
    assert _mask_root(proc.stderr) == _mask_root((exp / (stem + triple[1])).read_bytes()), (
        f"{rel}: STDERR pin drifted after the backfill (masked comparison)"
    )


if __name__ == "__main__":
    sys.exit(pytest.main([__file__, "-q"]))
