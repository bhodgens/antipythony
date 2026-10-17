"""C-02 traceability gate (docs/compat-contract.md:72).

Contract: "A CI check scans corpus case headers: every cited divergence id
exists." The existing per-suite lint only asserts a '# Cites:' header is
PRESENT; nothing validated that the cited ids actually exist in the
contract or the spec pack. This test closes that gap: every id cited in a
corpus case header must resolve to a defined clause (C-NN in the contract,
S-NN-NN in docs/spec/, A-N amendment, D-N divergence register).
"""

import re
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "tests" / "corpus"
CONTRACT = REPO / "docs" / "compat-contract.md"
SPEC_DIR = REPO / "docs" / "spec"

# Citation grammar in corpus headers: '# Cites: <ids>' where ids are
# comma/space separated; ranges 'C-19..C-23' and wildcards 'S-06-*' allowed.
CITES_RE = re.compile(r"^#\s*Cites:\s*(.+)$", re.MULTILINE)
ID_RE = re.compile(r"([CSA]-\d+(?:-\d+)?|D\d+)\.\.?\.?([CSA]-\d+(?:-\d+)?|D\d+)?|\b([CSA]-\d+(?:-\d+)?|D\d+)(-\*)?\b")


def _defined_ids() -> set[str]:
    """Every clause id defined by the contract, the spec pack, or amendments."""
    ids: set[str] = set()
    contract_text = CONTRACT.read_text(encoding="utf-8")
    ids.update(re.findall(r"\bC-\d{2}\b", contract_text))
    ids.update(re.findall(r"\bD\d+\b", contract_text))
    ids.update(re.findall(r"\bA-\d+\b", contract_text))
    for spec in SPEC_DIR.glob("*.md"):
        text = spec.read_text(encoding="utf-8")
        ids.update(re.findall(r"\bS-\d{2}-\d+\b", text))
        ids.update(re.findall(r"\bA-\d+\b", text))
    return ids


def _cited_ids(case: Path) -> list[str]:
    header = case.read_text(encoding="utf-8")[:1200]
    m = CITES_RE.search(header)
    if not m:
        return []
    line = m.group(1)
    out: list[str] = []
    # Ranges first (C-19..C-23), then wildcards (S-06-*), then bare ids.
    for rng in re.finditer(r"\b([A-Z]+)-(\d+)(?:-\d+)?\s*\.\.\s*([A-Z]+)-(\d+)\b", line):
        prefix, lo, hi = rng.group(1), int(rng.group(2)), int(rng.group(4))
        width = 2 if prefix == "C" else len(rng.group(2))
        for n in range(lo, hi + 1):
            out.append(f"{prefix}-{str(n).zfill(width)}")
    line_no_ranges = re.sub(r"\b[A-Z]+-\d+(?:-\d+)?\s*\.\.\s*[A-Z]+-\d+\b", " ", line)
    for wild in re.finditer(r"\b([A-Z]+)-(\d+)-\*\b", line_no_ranges):
        out.append(f"{wild.group(1)}-{wild.group(2)}")  # family prefix
    line_no_wild = re.sub(r"\b[A-Z]+-\d+-\*\b", " ", line_no_ranges)
    out.extend(re.findall(r"\b(?:C-\d{2}|S-\d{2}-\d+|A-\d+|D\d+)\b", line_no_wild))
    return out


def test_every_cited_id_exists():
    defined = _defined_ids()
    assert defined, "no clause ids parsed from contract/spec - parser broken"
    bad: list[str] = []
    n_cases = 0
    for case in CORPUS.rglob("*.py"):
        cited = _cited_ids(case)
        if cited:
            n_cases += 1
        for cid in cited:
            if cid in defined:
                continue
            # Wildcard families (S-06-*) resolve if ANY id in the family exists.
            if re.fullmatch(r"[CSA]-\d+", cid) and any(d.startswith(cid + "-") for d in defined):
                continue
            bad.append(f"{case.relative_to(REPO)}: cited id {cid} is not defined")
    assert n_cases > 100, f"traceability scan found only {n_cases} citing cases - scan broken"
    assert not bad, "stale corpus citations:\n" + "\n".join(bad)
