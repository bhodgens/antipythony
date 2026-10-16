# T-01 valid fixture: import statement forms (parse surface only — no resolution at 01.01).
# Cites: S-04-06 (import a.b.c binds top name), S-04-07 (from m import n), S-04-08 (as rebinding), S-04-09 (import inside function body).
import os
import os.path
import json.decoder as jd
from os import path as p, sep
from collections.abc import Iterable
from typing import (
    Any,
    Optional as Opt,
)
def local_import():
    import math
    return math.pi
print(os, jd, p, sep, Iterable, Any, Opt)
