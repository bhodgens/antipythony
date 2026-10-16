# T-01 valid fixture: multiple with-items, parenthesized across lines (3.9+ grammar).
# Cites: S-02-40/41 (enter/exit protocol, nesting order); plan-01 Stmt list names `with`.
from contextlib import suppress
class Res:
    def __enter__(self):
        return 1
    def __exit__(self, *exc):
        return False
with Res() as a, Res() as b:
    ab = a + b
with (
    Res() as c,
    Res() as d,
):
    cd = c + d
with suppress(Exception):
    pass
print(ab, cd)
