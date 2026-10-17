# Cites: S-05-4 (rows fnmatch, glob); GAP-1: fnmatch.filterfalse absent on reference
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import fnmatch
import glob
names_f = ["fnmatch", "fnmatchcase", "filter", "translate"]
names_g = ["glob", "iglob", "escape", "translate"]
print(all(hasattr(fnmatch, n) for n in names_f),
      all(hasattr(glob, n) for n in names_g))
print(hasattr(fnmatch, "filterfalse"), hasattr(fnmatch, "filter"))
print(fnmatch.fnmatch("Report.TXT", "*.txt"),
      fnmatch.fnmatchcase("Report.TXT", "*.txt"))
print(sorted(fnmatch.filter(["a.py", "b.txt", "c.py"], "*.py")))
print(repr(fnmatch.translate("*.py?")))
print(repr(fnmatch.translate("a[bc]d")))
print(glob.escape("a[b*"), repr(glob.escape("[a")), callable(glob.translate))
gt = glob.translate("docs/*.md")
print(type(gt).__name__ != "", len(gt))
if hasattr(gt, "drive"):
    print(gt.pattern is not None)
print(sorted(fnmatch.filter(["x1", "y2", "z3"], "[yz]?")))
print(sorted(fnmatch.filter(["a.py", "b.so"], "*.py")))
