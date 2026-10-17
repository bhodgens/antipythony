import sys, traceback
print("VERSION:", sys.version)

# [2] nested tuple except targets
try:
    1/0
except (ValueError, (TypeError, ZeroDivisionError)):
    print("caught")
except Exception as e:
    print("probe2 compile-time or runtime:", type(e).__name__, str(e))
