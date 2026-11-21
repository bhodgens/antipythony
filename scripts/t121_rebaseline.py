#!/usr/bin/env python3
"""T121 RE-BASELINE harness (leaf T121c, user decision C).

The pytest gate (tests/t10_threading::test_scaling_gate_3x_on_4cores_antipythony)
runs T121 with a ~0.11s release serial leg, best-of-3, and measures a RATIO —
thread spawn/park/join fixed cost dominates, and until 2026-11-20 nobody pinned
WHICH build profile it measured (the 3.31x record was DEBUG; release never
exceeded ~1.06x). This harness is the re-baseline methodology from
T121-REBASELINE.md:

  1. PROFILE-PINNED: measures a debug AND a release runner explicitly; the
     gate number is the RELEASE number (the shipped artifact).
  2. SIZED WORKLOAD: T10_REBASELINE_N (default 750_000) puts the SERIAL leg
     at ~1.4s release, so fixed spawn/join cost is <2% of the leg.
  The old small workload (N=60000, serial ~0.11s) is ALSO recorded for
     continuity with the 02.09b / bisect numbers.
  3. QUIET-WINDOW INTERLEAVED A/B/A: each round is serial, parallel, serial
     (defeats thermal/load drift); best-of-R per leg per workload; the
     harness refuses to start (and re-checks per round) when the 1-minute
     load average is above the quiet threshold.
  4. THREAD-COUNT SENSITIVITY: NSH is fixed at 4 (the pinned workload shape);
     thread-count sensitivity is deferred — see T121-REBASELINE.md gaps.

No interpreter code is exercised beyond `antipythony run`; this script is
measurement-only. Corpus rule compliance: no sleep(), no banned imports.

Usage:
  python3 scripts/t121_rebaseline.py [--rounds 5] \
      [--threads 750000 60000] [--min-load 2.0] \
      [--out plans/antipythony/02-native-backend/T121-REBASELINE-raw.json]
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
CASE = REPO / "tests" / "corpus" / "threading" / "T121-scaling-parallel-cpu.py"


def load1():
    try:
        with open("/proc/loadavg") as f:
            return float(f.read().split()[0])
    except OSError:
        out = subprocess.run(["uptime"], capture_output=True, text=True).stdout
        m = re.search(r"load averages?: ([0-9.]+)", out)
        return float(m.group(1)) if m else float("nan")


def one_leg(runner, mode, nenv):
    env = dict(os.environ)
    env["T10_MODE"] = mode
    env.update(nenv)
    t0 = time.perf_counter()
    proc = subprocess.run(runner + [str(CASE)], capture_output=True,
                          timeout=1800, cwd=str(REPO), env=env)
    wall = time.perf_counter() - t0
    assert proc.returncode == 0, (mode, proc.stderr[-400:])
    m = re.search(r"%s ([0-9.]+)" % mode, proc.stderr.decode())
    assert m, proc.stderr.decode()[-200:]
    return float(m.group(1)), wall


def measure_round(runner, n_iter, rounds):
    """Interleaved A/B/A: rounds x (serial, parallel, serial). Best-of per leg."""
    nenv = {"T10_REBASELINE_N": str(n_iter)}
    serials, parallels = [], []
    for i in range(rounds):
        load = load1()
        assert load < args_min_load[0], (
            f"round {i+1}: load {load} >= {args_min_load[0]} — wait for a quiet window")
        s1, _ = one_leg(runner, "serial", nenv)
        p, _ = one_leg(runner, "parallel", nenv)
        s2, _ = one_leg(runner, "serial", nenv)
        serials += [s1, s2]
        parallels.append(p)
        print(f"    round {i+1}/{rounds}: serial {min(s1,s2):.4f}s "
              f"parallel {p:.4f}s (load {load:.1f})", flush=True)
    return min(serials), min(parallels), serials, parallels


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--threads", type=int, nargs="+", default=[750000, 60000],
                    help="workload iteration counts (T10_REBASELINE_N); first is the sized leg")
    ap.add_argument("--min-load", type=float, default=2.0)
    ap.add_argument("--profile", choices=["release", "debug", "both"], default="both")
    ap.add_argument("--out", default="")
    args = ap.parse_args()
    global args_min_load
    args_min_load = [args.min_load]

    runners = {}
    for profile, rel in (("release", "target-0304c/release/antipythony"),
                         ("debug", "target-0304c/debug/antipythony")):
        p = REPO / rel
        if p.is_file():
            runners[profile] = [str(p), "run"]
    assert "release" in runners, "build release first: cargo build --release -p ap-cli"

    start_load = load1()
    assert start_load < args.min_load, (
        f"load {start_load} >= {args.min_load} — this harness only measures in quiet windows")

    results = {"host": os.uname().nodename, "cores": os.cpu_count(),
               "start_load": start_load, "profiles": {}, "commit": ""}
    try:
        results["commit"] = subprocess.run(
            ["git", "rev-parse", "--short", "HEAD"], cwd=REPO,
            capture_output=True, text=True).stdout.strip()
    except Exception:
        pass

    wanted = list(runners) if args.profile == "both" else [args.profile]
    for profile in wanted:
        if profile not in runners:
            continue
        runner = runners[profile]
        results["profiles"][profile] = {}
        for n in args.threads:
            print(f"[{profile}] workload N={n}", flush=True)
            s, p, sraw, praw = measure_round(runner, n, args.rounds)
            ratio = s / p
            results["profiles"][profile][f"N={n}"] = {
                "serial_best": s, "parallel_best": p, "ratio": round(ratio, 4),
                "serial_all": sraw, "parallel_all": praw,
            }
            print(f"  -> serial {s:.4f}s parallel {p:.4f}s RATIO {ratio:.3f}x", flush=True)
            if args.out:
                Path(args.out).write_text(json.dumps(results, indent=2))

    gate = results["profiles"].get("release", {}).get(f"N={args.threads[0]}")
    if gate:
        print(f"\nGATE NUMBER (release, sized leg N={args.threads[0]}): "
              f"{gate['ratio']}x  (serial {gate['serial_best']:.4f}s / "
              f"parallel {gate['parallel_best']:.4f}s)")
    else:
        print(f"\n(profile {args.profile}: no release gate number in this run)")
    if args.out:
        Path(args.out).write_text(json.dumps(results, indent=2))
        print(f"raw JSON: {args.out}")


if __name__ == "__main__":
    main()
