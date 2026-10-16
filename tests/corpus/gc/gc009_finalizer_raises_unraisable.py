# Cites: C-21, C-22, C-23, D1
# A __del__ that raises during gc.collect() produces the reference's
# unraisable report and does NOT change the exit code (C-29/C-30: rc 0).
# Two stable projections are pinned:
#  (1) the stderr text captured by swapping sys.stderr around the collect,
#      MASKED for everything unspecified: the function object address is
#      replaced (0x... -> 0xADDR) and the traceback frame line is replaced
#      by its shape (`  File ...__del__` -> `FRAME`), since the invoking
#      script path text is a harness concern (S-06-17#4, C-24: addresses
#      and paths-in-tracebacks are never raw-diffed). What remains pinned:
#      the `Exception ignored while calling deallocator <function
#      Bad.__del__ at 0xADDR>:` opener (C-21's mandated stderr shape), the
#      `Traceback (most recent call last):` line, the indented source-line
#      echo, and the final `ValueError: shape-check` line.
#  (2) the structured args handed to a custom sys.unraisablehook — type,
#      message, traceback presence, object-field type — no addresses.
import gc
import io
import re
import sys

buf = io.StringIO()
saved_stderr = sys.stderr

class Bad:
    def __del__(self):
        raise ValueError("shape-check")

sys.stderr = buf
b = Bad()
del b
gc.collect()
sys.stderr = saved_stderr
text = buf.getvalue()
masked = re.sub(r"0x[0-9a-fA-F]+", "0xADDR", text)
masked = re.sub(r'^  File ".*"', "FRAME", masked, flags=re.M)
lines = masked.splitlines()
print("line_count:", len(lines))
print("line0:", lines[0])
print("line1:", lines[1])
print("frame slot:", lines[2])
print("source echo:", repr(lines[3]))
print("last:", lines[-1])
print("starts-with-form:", masked.startswith(
    "Exception ignored while calling deallocator <function Bad.__del__ at 0xADDR>:\n"
))

records = []
def hook(args):
    records.append((args.exc_type.__name__, str(args.exc_value),
                   args.exc_traceback is not None, type(args.object).__name__))
old_hook = sys.unraisablehook
sys.unraisablehook = hook

class Bad2:
    def __del__(self):
        raise RuntimeError("boom-del")

b2 = Bad2()
del b2
gc.collect()
print("hook records:", records)
sys.unraisablehook = old_hook
print("rc stays 0 through unraisable finalizers")
