# T-01 valid fixture: `from m import *` at module level is VALID (the reject case is in-func — S-04-10).
# Cites: S-04-10 (import * only allowed at module level); spec 04 anchor A10 (star import binding rules).
from string import *
from collections import *
print(ascii_lowercase, namedtuple)
