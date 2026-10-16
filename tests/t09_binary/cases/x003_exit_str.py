# T-09 exit-code case (gates 02.06 AoT binary)
# Cites: C-28 (non-int -> str(x) on stderr as sole output, rc 1), S-06-11
import sys
sys.exit("boom")
