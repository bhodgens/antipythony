"""PP2-tier1 harness: run pp2_tier1_dispatch.py under tier-0 vs adaptive, N times.

Writes pp2_tier1_report.json next to this file with per-shape median deltas
and the tier-1 install/exercise stats from stderr.
"""
import json
import statistics
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[3]
BIN = REPO / "target-pp2-tier1" / "release" / "ap-run-eval"
PROBE = HERE / "pp2_tier1_dispatch.py"
REPEATS = 7


def run(mode):
    """One probe run under `mode`; returns (json_out, tier1_installed_set)."""
    env = {"PATH": "/usr/bin:/bin", "APX_TIER_STATS": "1", "APX_TIER1_STATS": "1"}
    p = subprocess.run(
        [str(BIN), "--tier", mode, str(PROBE)],
        capture_output=True, text=True, env=env, timeout=120,
    )
    if p.returncode != 0:
        raise SystemExit(f"probe failed rc={p.returncode}: {p.stderr[-500:]}")
    installed = set()
    for line in p.stderr.splitlines():
        if line.startswith("[TIER1-STATS] install fn="):
            installed.add(line.split("fn=")[1].split()[0])
    return json.loads(p.stdout), installed


def main():
    samples = {"0": [], "adaptive": []}
    installs = set()
    for _ in range(REPEATS):
        for mode in ("0", "adaptive"):
            out, ins = run(mode)
            samples[mode].append(out)
            installs |= ins

    shapes = [k for k in samples["0"][0] if k != "tier_mode"]
    rows = {}
    for shape in shapes:
        t0 = [r[shape]["best_s"] for r in samples["0"]]
        t1 = [r[shape]["best_s"] for r in samples["adaptive"]]
        m0, m1 = statistics.median(t0), statistics.median(t1)
        results_match = {r[shape]["result"] for r in samples["0"]} == {
            r[shape]["result"] for r in samples["adaptive"]}
        rows[shape] = {
            "tier0_med_ms": round(m0 * 1e3, 3),
            "adaptive_med_ms": round(m1 * 1e3, 3),
            "speedup": round(m0 / m1, 2) if m1 else None,
            "tier0_ns_per_iter": round(m0 / 32768 * 1e9, 1),
            "adaptive_ns_per_iter": round(m1 / 32768 * 1e9, 1),
            "results_identical_across_modes": results_match,
        }

    report = {
        "probe": "pp2_tier1_dispatch.py",
        "repeats": REPEATS,
        "tier1_installed_fns": sorted(installs),
        "rows": rows,
    }
    (HERE / "pp2_tier1_report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    sys.exit(main())
