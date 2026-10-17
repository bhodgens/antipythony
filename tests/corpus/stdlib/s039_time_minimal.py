# Cites: S-05-4 (row time minimal): presence + deterministic UTC calendar only (value-bearing calls prohibited)
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import time
names = ["monotonic", "monotonic_ns", "perf_counter", "perf_counter_ns",
         "sleep", "time", "time_ns", "struct_time", "gmtime", "localtime",
         "mktime", "strftime", "strptime", "timezone", "clock_gettime",
         "get_clock_info", "CLOCK_MONOTONIC", "CLOCK_REALTIME"]
print(len(names), all(hasattr(time, n) for n in names))
print(callable(time.monotonic), callable(time.perf_counter),
      callable(time.sleep), callable(time.time))
gt = time.gmtime(0)
print(gt)
print(gt.tm_year, gt.tm_mon, gt.tm_mday, gt.tm_hour, gt.tm_min, gt.tm_sec,
      gt.tm_wday, gt.tm_yday, gt.tm_isdst)
print(tuple(gt))
print(time.strftime("%Y-%m-%d %H:%M:%S", gt))
print(time.strptime("2026-10-16", "%Y-%m-%d").tm_year)
st = time.struct_time((2020, 1, 2, 3, 4, 5, 3, 2, 0))
print(time.strftime("%Y-%m-%d %H:%M:%S", st))
print(time.mktime(time.strptime("2020-01-02", "%Y-%m-%d")) > 1_500_000_000)
print(callable(time.time_ns), time.CLOCK_MONOTONIC is not None,
      callable(time.clock_gettime), time.get_clock_info("monotonic").monotonic)
print(time.sleep(0) is None)
print(repr(tuple(gt)), gt.n_fields, gt.n_sequence_fields, gt.n_unnamed_fields)
print(callable(time.localtime), callable(time.gmtime))
