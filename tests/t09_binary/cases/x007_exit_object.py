# T-09 exit-code case (gates 02.06 AoT binary)
# Cites: C-28 (non-int payload: raw str() used, not repr of __str__-definer)
import sys

class S:
    def __str__(self):
        return "custom-str"

sys.exit(S())
