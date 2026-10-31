# Cites: C-21, C-23, C-24, D1
# ISOLATED fence for the gc009 failure shape (issue #10): the unraisable
# report's source-line echo. CPython strips the leading indentation of the
# raising source line and re-indents 4 spaces ("    raise ..."), regardless
# of the def's nesting depth. The fence nests __del__ 3 levels deep so the
# raw-indent echo (impl bug: 4 + unstripped source) diverges maximally from
# the reference form. Everything unspecified is MASKED (address -> 0xADDR,
# File line -> FRAME) per S-06-17#4/C-24; what remains pinned is the
# stripped source echo and the final exception line.
import gc
import io
import re
import sys

buf = io.StringIO()
saved_stderr = sys.stderr

class Outer:
    class Inner:
        class Deep:
            def __del__(self):
                raise ValueError("echo-strip-check")

sys.stderr = buf
d = Outer.Inner.Deep()
del d
gc.collect()
sys.stderr = saved_stderr
text = buf.getvalue()
masked = re.sub(r"0x[0-9a-fA-F]+", "0xADDR", text)
masked = re.sub(r'^  File ".*"', "FRAME", masked, flags=re.M)
lines = masked.splitlines()
print("line_count:", len(lines))
print("line0:", lines[0])
print("frame slot:", lines[2])
print("source echo:", repr(lines[3]))
print("last:", lines[-1])
