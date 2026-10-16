# T-09 exit-code case (gates 02.06 AoT binary)
# Cites: C-28 (SystemExit in non-main thread ends that thread, does NOT set
#        process exit code; only main-thread SystemExit does)
import _thread
import sys
import time

def g():
    print("in-thread")
    sys.exit(5)

_thread.start_new_thread(g, ())
time.sleep(0.2)
print("main-done")
