# T-09 exit-code case (gates 02.06 AoT binary)
# Cites: C-28, C-30, S-06-11 (int row: sys.exit(n) -> n & 0xFF)
import sys
sys.exit(2)
