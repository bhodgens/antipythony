#!/usr/bin/env python3
"""Generate a speedup-vs-CPython chart from bench_report.json.

Every bar length and label is computed from the JSON. No number is
hand-transcribed. Sorted by ratio (best first); 1.0x is the CPython baseline.
"""
import argparse
import json
import math
from pathlib import Path

HERE = Path(__file__).resolve().parent
DOC = HERE / "../../../docs"
ROW_H = 18
LABEL_W = 330
PLOT_W = 520
SMALL = 0.005
LARGE = 40.0
GREEN = "#2e8b57"
RED = "#c0392b"
GREY = "#888888"


def bar_len(ratio):
    lo, hi = math.log10(SMALL), math.log10(LARGE)
    v = max(math.log10(max(ratio, SMALL)), lo)
    return (v - lo) / (hi - lo) * PLOT_W


def esc(s):
    return (s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--report", default=str(HERE / "bench_report.json"))
    ap.add_argument("--out", default=str(DOC / "fast-path-speedup-vs-cpython.svg"))
    args = ap.parse_args()
    rep = json.loads(Path(args.report).read_text())
    rows = [r for r in rep["rows"] if r.get("antipythony_gbs") and r.get("ratio_vs_cpython")]
    rows.sort(key=lambda r: -r["ratio_vs_cpython"])
    y0 = 76
    height = y0 + len(rows) * ROW_H + 44
    parts = [
        '<?xml version="1.0" encoding="UTF-8"?>',
        '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" '
        'viewBox="0 0 %d %d" font-family="Helvetica,Arial,sans-serif">'
        % (LABEL_W + PLOT_W + 170, height, LABEL_W + PLOT_W + 170, height),
        "<style>.l{font-size:11px}.v{font-size:11px;fill:#333}.b{font-size:12px}</style>",
        '<rect width="100%" height="100%" fill="white"/>',
        '<text x="12" y="26" font-size="16" font-weight="bold">'
        "antipythony vs CPython 3.14.7 - speedup by primitive (log scale)</text>",
        '<text x="12" y="44" font-size="11" fill="#555">Source: tests/bench/primitive/bench_report.json '
        "(generated %s, %s). Bars: x times faster than CPython; right of 1.0x is faster.</text>"
        % (esc(rep["generated"]), esc(rep["machine"])),
    ]
    one_x = LABEL_W + bar_len(1.0)
    parts.append('<line x1="%.1f" y1="%d" x2="%.1f" y2="%d" stroke="%s" stroke-dasharray="4,4"/>'
                 % (one_x, y0 - 6, one_x, y0 + len(rows) * ROW_H, GREY))
    parts.append('<text x="%.1f" y="%d" font-size="10" fill="#555">1.0x CPython</text>'
                 % (one_x + 3, y0 - 8))
    for i, r in enumerate(rows):
        y = y0 + i * ROW_H
        ratio = r["ratio_vs_cpython"]
        label = "%s  %s" % (r["primitive"], r["size"])
        parts.append('<text x="10" y="%d" class="l">%s</text>' % (y + 9, esc(label)))
        w = bar_len(ratio)
        color = GREEN if ratio > 1 else RED
        parts.append('<rect x="%d" y="%d" width="%.1f" height="11" fill="%s"/>'
                     % (LABEL_W, y, w, color))
        parts.append('<text x="%.1f" y="%d" class="v">%.2fx (%s)</text>'
                     % (LABEL_W + w + 5, y + 9, ratio, r["verdict"]))
    parts.append('<text x="10" y="%d" font-size="11" fill="#333">'
                 "Green = faster than CPython. Red = slower. "
                 "PASS/FAIL is the benchmark target gate, not the CPython ratio.</text>"
                 % (y0 + len(rows) * ROW_H + 24))
    parts.append("</svg>")
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(parts) + "\n")
    print("wrote", out, "rows", len(rows))


if __name__ == "__main__":
    main()
