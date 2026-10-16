# T-05 values corpus V067
# Cites: C-14, S-01-1
# Pins: the int->str conversion digit limit raises ValueError with the exact
# measured text (str() form has no digit-count clause; int() form names the
# count); raising the limit via sys.set_int_max_str_digits makes it work;
# 2**10000 (3011 digits) prints fully without touching the limit.
import sys
try:
    str(10**5000)
except Exception as e:
    print(type(e).__name__ + ":", e)
try:
    int("1" * 5000)
except Exception as e:
    print(type(e).__name__ + ":", e)
sys.set_int_max_str_digits(6000)
s = str(10**5000)
print(len(s), s[:3], s[-3:], int(s) == 10**5000)
print(len(str(2**10000)))
