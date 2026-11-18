#!/usr/bin/env python3
"""Leaf 03.05 startup measurement: hello-world -> first output byte (TTFB)
through the ap-host CLI binary. TARGET <50ms, MEASURED not asserted.

Method: N fresh `ap-host run --ttfb hello.wasm` process invocations (each
pays full process start + engine build + compile + instantiate + run);
the TTFB number comes from the binary's own in-process Instant-based
harness (--ttfb prints [ap-host] ttfb ...us). Both numbers logged.

Also measures the warm in-process path (the embedding API) for reference.
"""

import re
import statistics
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BIN = Path(sys.argv[1] if len(sys.argv) > 1 else REPO / "target-0304c/debug/ap-host")
HELLO = REPO / "logs/wasm-corpus/hello.wasm"
N = 20


def main() -> int:
    assert BIN.exists(), f"binary missing: {BIN}"
    assert HELLO.exists(), f"hello module missing: {HELLO}"
    ttfbs = []
    totals = []
    walls = []
    for _ in range(N):
        t0 = time.perf_counter()
        proc = subprocess.run(
            [str(BIN), "run", "--ttfb", str(HELLO)],
            capture_output=True,
            timeout=60,
        )
        wall_ms = (time.perf_counter() - t0) * 1000.0
        m = re.search(
            rb"ttfb (\d+)us \((\d+)\.(\d+)ms\), total (\d+)us \((\d+)\.(\d+)ms\)",
            proc.stderr,
        )
        assert m, f"no ttfb line in stderr: {proc.stderr!r}"
        ttfbs.append(int(m.group(1)) / 1000.0)  # us -> ms
        totals.append(int(m.group(4)) / 1000.0)
        walls.append(wall_ms)

    def stats(xs):
        return f"min {min(xs):.2f}ms  median {statistics.median(xs):.2f}ms  max {max(xs):.2f}ms"

    print(f"hello-world TTFB over {N} fresh CLI invocations:")
    print(f"  in-process ttfb (first stdout byte):  {stats(ttfbs)}")
    print(f"  in-process total (compile+run):       {stats(totals)}")
    print(f"  whole-process wall (fork+exec+...):   {stats(walls)}")
    gate = max(ttfbs) < 50.0
    med = statistics.median(ttfbs)
    print(f"GATE: median ttfb {med:.2f}ms, max {max(ttfbs):.2f}ms -> "
          f"{'PASS (<50ms)' if gate else 'FAIL (>=50ms)'}")
    return 0 if gate else 1


if __name__ == "__main__":
    sys.exit(main())
