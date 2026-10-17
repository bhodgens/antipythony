# Cites: S-05-4 (row traceback), C-25 (frame-path-free pins only)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import sys
import traceback
names = ["print_exc", "format_exc", "format_exception",
         "format_exception_only", "print_exception", "format_tb", "print_tb",
         "format_list", "print_list", "format_stack", "print_stack",
         "extract_tb", "extract_stack", "walk_tb", "walk_stack",
         "TracebackException", "StackSummary", "FrameSummary",
         "clear_frames", "suppress"]
print(len(names), all(hasattr(traceback, n) for n in names))
print(traceback.format_exception_only(ValueError("boom")))
print(traceback.format_exception_only(TypeError("bad", "args")))
try:
    1 / 0
except ZeroDivisionError:
    typ, val, tb = sys.exc_info()
    print([ln.rstrip("\n") for ln in
           traceback.format_exception_only(typ, val)])
    print(hasattr(tb, "tb_frame"))
    frames = list(traceback.walk_tb(tb))
    print(len(frames) >= 1, frames[0] is tb)
    print(traceback.TracebackException is not None,
          traceback.StackSummary is not None)
    print(traceback.extract_tb is not None,
          len(traceback.format_tb(tb)) == len(frames))
    se = traceback.TracebackException(typ, val, tb)
    print(se.exc_type_str == "ZeroDivisionError",
          len(se.stack) == len(frames), se.exceptions is None)
try:
    raise KeyError("nope")
except KeyError:
    print("".join(traceback.format_exception(*sys.exc_info())).count("KeyError: 'nope'"))
print(callable(traceback.clear_frames), hasattr(traceback, "suppress"))
print(issubclass(traceback.StackSummary, list),
      issubclass(traceback.FrameSummary, object))
