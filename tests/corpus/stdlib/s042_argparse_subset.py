# Cites: S-05-4 (row argparse): fixed argv, prog pinned, no help reflow (corpus-prohibited per row)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import argparse
names = ["ArgumentParser", "Namespace", "Action", "ArgumentError",
         "ArgumentTypeError"]
print(len(names), all(hasattr(argparse, n) for n in names))
p = argparse.ArgumentParser(prog="apx")
p.add_argument("pos")
p.add_argument("--n", type=int, default=0)
p.add_argument("--name", dest="nm", default="x")
p.add_argument("-v", action="count", default=0)
p.add_argument("--mode", choices=["fast", "slow"], default="fast")
ns = p.parse_args(["hello", "--n", "7", "--mode", "slow", "-vv"])
print(ns)
print(ns.pos, ns.n + 1, ns.nm, ns.v, ns.mode)
try:
    p.parse_args(["hello", "--n", "x"])
except SystemExit as e:
    print("exit-code:", e.code)
sp = argparse.ArgumentParser(prog="apx2", exit_on_error=False)
sp.add_argument("--n", type=int)
try:
    sp.parse_args(["--n", "x"])
except argparse.ArgumentError as e:
    print("ArgumentError:", e)
known, extra = p.parse_known_args(["a", "--unknown-flag"])
print(known.pos, sorted(extra))
sub = argparse.ArgumentParser(prog="apx3")
subp = sub.add_subparsers(dest="cmd", required=True)
addp = subp.add_parser("add")
addp.add_argument("items", nargs="+")
r = sub.parse_args(["add", "1", "2"])
print(r.cmd, r.items)
print(isinstance(argparse.Namespace(a=1), argparse.Namespace),
      argparse.Namespace(a=1) == argparse.Namespace(a=1))
print(issubclass(argparse.Action, object),
      issubclass(argparse.ArgumentError, Exception),
      issubclass(argparse.ArgumentTypeError, Exception))
try:
    sub.parse_args(["rm"])
except SystemExit as e:
    print("sub-required-exit:", e.code)
