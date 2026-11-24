#!/usr/bin/env python3
"""Primitive benchmark harness: run probes under CPython + antipythony.

Emits bench_report.json per the frozen schema in
plans/primitive-perf/00-harness/orchestrator.md (Interface contract).
"""
import argparse
import datetime
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROBES = HERE / "probes"

# id -> (probe file, primitive label prefix)
PROBE_FILES = {
    "P01": "p01_base64.py",
    "P03": "p03_hexlify.py",
    "P04": "p04_compress.py",
    "P05": "p05_hash.py",
    "P06": "p06_str.py",
    "P07": "p07_bytes_int.py",
    "P08": "p08_sort.py",
    "P09": "p09_dict.py",
    "P10": "p10_json.py",
}

# Only these two primitives have an upstream SOTA number.
B64_TARGETS = {
    "base64.b64encode": (24.0, "CPython PR #162443 (NEON)"),
    "base64.b64decode": (14.0, "CPython PR #162443 (NEON)"),
}

TIMEOUT = 120  # seconds per probe run
LINE_RE = re.compile(r"^\{.*\}$")


def parse_probe_output(stdout):
    """Parse JSON lines; return dict (probe,size)->row, skipping comments."""
    out = {}
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue  # skip notes/guards
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if "probe" in obj and "best_s" in obj:
            out[(obj["probe"], obj["size"])] = obj
    return out


def size_label(nbytes):
    if nbytes % 1048576 == 0:
        return "%dMiB" % (nbytes // 1048576)
    if nbytes % 1024 == 0:
        return "%dKiB" % (nbytes // 1024)
    return "%dB" % nbytes


def run_probe(cmd, timeout=TIMEOUT):
    """Run one probe; return (results_dict, None) or (None, stderr_tail)."""
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True,
                              timeout=timeout)
    except subprocess.TimeoutExpired:
        return None, "TIMEOUT after %ds" % timeout
    except OSError as e:
        return None, "launch failed: %s" % e
    if proc.returncode != 0:
        tail = "\n".join(proc.stderr.splitlines()[-5:])
        return None, "rc=%d stderr: %s" % (proc.returncode, tail)
    results = parse_probe_output(proc.stdout)
    if not results:
        tail = "\n".join((proc.stdout + proc.stderr).splitlines()[-5:])
        return None, "no JSON lines parsed. tail: %s" % tail
    return results, None


def interp_version(cmd):
    try:
        proc = subprocess.run(cmd + ["-V"] if cmd != ["antipythony"] else cmd,
                              capture_output=True, text=True, timeout=30)
        return (proc.stdout + proc.stderr).strip().splitlines()[-1]
    except Exception:
        return "unknown"


def main():
    ap = argparse.ArgumentParser(description="primitive bench harness")
    ap.add_argument("--antipythony", required=True,
                    help="path to antipythony binary")
    ap.add_argument("--python", default=sys.executable,
                    help="CPython reference interpreter")
    ap.add_argument("--out", default=str(HERE / "bench_report.json"))
    ap.add_argument("--sizes", default="48KiB,1MiB",
                    help="comma list: 48KiB,1MiB (informational filter)")
    ap.add_argument("--only", default=None,
                    help="comma list of probe ids, e.g. P01,P03")
    args = ap.parse_args()

    wanted = None
    if args.only:
        wanted = [s.strip().upper() for s in args.only.split(",") if s.strip()]
    sizes_wanted = [s.strip() for s in args.sizes.split(",") if s.strip()]

    py_cmd = [args.python]
    ap_cmd = [args.antipythony, "run"]

    rows = []
    for pid in sorted(PROBE_FILES):
        if wanted and pid not in wanted:
            continue
        probe_path = PROBES / PROBE_FILES[pid]
        print("RUN %s: %s" % (pid, probe_path.name))
        py_res, py_err = run_probe(py_cmd + [str(probe_path)])
        ap_res, ap_err = run_probe(ap_cmd + [str(probe_path)])
        if py_res is None:
            print("WARN %s: CPython run failed: %s" % (pid, py_err),
                  file=sys.stderr)
            continue
        for (primitive, nbytes), pr in py_res.items():
            lbl = size_label(nbytes)
            # Bucket non-exact payloads (e.g. p10_json's nested structure is
            # 71558B) into the nearest harness size so the --sizes filter
            # and the frozen schema's size labels stay consistent.
            if lbl not in ("48KiB", "1MiB"):
                lbl = "48KiB" if nbytes < 524288 else "1MiB"
            if sizes_wanted and lbl not in sizes_wanted:
                continue
            cpy_s = pr["best_s"]
            cpy_gbs = pr["gbs"]
            if ap_res is not None and (primitive, nbytes) in ap_res:
                ap_s = ap_res[(primitive, nbytes)]["best_s"]
                ap_gbs = ap_res[(primitive, nbytes)]["gbs"]
                err_tail = None
            else:
                ap_s = None
                ap_gbs = None
                err_tail = ap_err or "primitive %r missing from antipythony output" % primitive
            tgt, src = B64_TARGETS.get(primitive, (None, None))
            if tgt is None:
                tgt = cpy_gbs
                src = "beat CPython as shipped"
            row = {
                "id": pid,
                "primitive": primitive,
                "size": lbl,
                "cpython_s": cpy_s,
                "antipythony_s": ap_s,
                "cpython_gbs": cpy_gbs,
                "antipythony_gbs": ap_gbs,
                "target_gbs": tgt,
                "target_source": src,
                "ratio_vs_cpython": round(ap_gbs / cpy_gbs, 3) if ap_gbs else None,
                "ratio_vs_target": round(ap_gbs / tgt, 4) if ap_gbs else None,
                "verdict": None,
            }
            if ap_gbs is None:
                row["verdict"] = "ERROR"
                row["error"] = err_tail
            elif ap_gbs >= tgt:
                row["verdict"] = "PASS"
            else:
                row["verdict"] = "FAIL"
            rows.append(row)

    report = {
        "generated": datetime.datetime.now(datetime.timezone.utc)
        .isoformat(timespec="seconds"),
        "machine": "Mac M2 Ultra",
        "python_ref": interp_version(py_cmd),
        "antipythony_build": "release",
        "rows": rows,
    }
    Path(args.out).write_text(json.dumps(report, indent=2) + "\n")

    # human summary
    print("\n== primitive bench summary ==")
    hdr = "%-5s %-24s %-8s %10s %10s %8s %7s %s"
    print(hdr % ("id", "primitive", "size", "cpy GB/s", "apy GB/s",
                 "vs cpy", "verdict", "err?"))
    for r in rows:
        print(hdr % (
            r["id"], r["primitive"][:24], r["size"],
            "%.4f" % r["cpython_gbs"],
            "n/a" if r["antipythony_gbs"] is None else "%.4f" % r["antipythony_gbs"],
            "n/a" if r["ratio_vs_cpython"] is None else "%.2fx" % r["ratio_vs_cpython"],
            r["verdict"], "yes" if r["verdict"] == "ERROR" else "-",
        ))
    print("\nwrote %s (%d rows)" % (args.out, len(rows)))


if __name__ == "__main__":
    main()
