# T-09 argv-probe case (gates 02.06 AoT binary)
# Cites: 02.06 leaf gate (`./app` UX), C-27 (stdout bytes); argv passthrough:
#        sys.argv in the binary = what is given after the binary name
import sys
print(sys.argv[1:])
