#!/usr/bin/env python3
"""Measure peak RSS of a child via wait4(rusage), then time the probe."""
import os, sys, time, resource

argv = sys.argv[1:]
t0 = time.time()
pid = os.fork()
if pid == 0:
    os.execvp(argv[0], argv)
_, status, ru = os.wait4(pid, 0)
print("rc", os.waitstatus_to_exitcode(status))
print("maxrss_kb", ru.ru_maxrss)
print("wall_s", round(time.time() - t0, 1))
