# T-01 valid fixture: except* exception-group syntax (3.11+ grammar; 3.14 pin includes it).
# Cites: C-03 (3.14 grammar surface); S-02-30 family (exception handling forms must parse).
import builtins
try:
    pass
except* TypeError as eg:
    n = len(eg.exceptions)
except* (ValueError, KeyError):
    n = 0
else:
    n = -1
finally:
    print(n)
