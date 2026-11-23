# Leaf 1006 differential driver part 1: PyLong import/export + GC bridging.
# Runs byte-identical under antipythony (longgc extension via build-ext)
# and CPython 3.14.7 (same source compiled against Python.h).
import longgc as m

DEF = -1          # Py_ASNATIVEBYTES_DEFAULTS
LE = 1            # Py_ASNATIVEBYTES_LITTLE_ENDIAN
UNS = 4           # Py_ASNATIVEBYTES_UNSIGNED_BUFFER (NOTE: also =BE bit trap)
REJ = 8           # Py_ASNATIVEBYTES_REJECT_NEGATIVE

# L0: the native layout (30-bit digits, LSB first, LE digits).
print("L0", m.layout())

# ---- PyLong_Export: compact arm ----
print("E1", m.export_compact(0))            # value=0 nd=0
print("E2", m.export_compact(-1))           # value=-1 nd=0 neg=0
print("E3", m.export_compact(123))          # value=123
print("E4", m.export_compact(-123))
print("E5", m.export_compact(2**62))        # fits i64: compact
print("E6", m.export_compact(-2**62))
print("E7", m.export_compact(2**63 - 1))    # INT64_MAX compact
print("E8", m.export_compact(-2**63))       # INT64_MIN compact
print("E9", m.export_compact(2**63))        # NON-compact: value=0 nd>=1

# ---- PyLong_Export: non-compact arm (the digit layouts) ----
print("D1", m.export_digits(10**30))        # [0, 433305513, 852086020, 807]
print("D2", m.export_digits(-(2**70)))      # [0, 0, 1024] neg=1
print("D3", m.export_digits(2**63))         # [0, 0, 8]
print("D4", m.export_digits(-(2**64)))      # [0, 0, 16] neg=1
print("D5", m.export_digits(2**111))
print("D6", m.export_digits(123456789012345678901234567890))
print("D7", m.export_digits(999999999999999999999999999999))
print("D8", m.export_digits(-(2**69)))

# ---- Export error path + FreeExport ----
try:
    m.export_compact("not an int")
except TypeError as e:
    print("X1", type(e).__name__, e)
print("X2", m.export_none())                 # rc=-1
try:
    m.export_compact(None)
except TypeError as e:
    print("X3", type(e).__name__, e)
print("F1", m.freeexport_twice())            # (0, 0)

# ---- PyLong_AsNativeBytes ----
# need-size (n=0)
print("N1", m.asnative_need(255, DEF))
print("N2", m.asnative_need(2**30, DEF))
print("N3", m.asnative_need(10**30, DEF))
print("N4", m.asnative_need(-(2**70), DEF))
print("N5", m.asnative_need(70000, DEF))
# fills
print("A1", m.asnative(255, 16, DEF))        # ff 00... (LE, sign-fill 00)
print("A2", m.asnative(-1, 16, DEF))         # all ff
print("A3", m.asnative(-2**70, 16, DEF))     # 00*8 c0 ff*7
print("A4", m.asnative(-2**70, 9, DEF))      # 00*8 c0
print("A5", m.asnative(255, 2, DEF))         # rc=2 ff 00
print("A6", m.asnative(-1, 1, DEF))          # rc=1 ff
print("A7", m.asnative(70000, 2, DEF))       # rc=8 (need), 70 11
print("A8", m.asnative(-5, 16, DEF))         # fb ff*15
print("A9", m.asnative(-128, 16, DEF))       # 80 ff*15
print("A10", m.asnative(-2**63, 16, DEF))    # 00*7 80 ff*8 (rc=9)
print("A11", m.asnative(2**100, 16, DEF))    # truncation, rc=13
print("A12", m.asnative(255, 16, UNS))       # flags=4 -> BIG endian trap
# errors
print("AR1", m.asnative(-(2**70), 16, REJ))  # ValueError (err=1, bytes EE)
print("AR2", m.asnative(255, -5, DEF))       # SystemError longobject.c:1215

# ---- PyLong_FromNativeBytes ----
print("FR1", m.fromnative([0xd2, 0x04, 0, 0, 0, 0, 0, 0], DEF, False))   # 1234
print("FR2", m.fromnative([0xd2], DEF, False))                            # -46
print("FR3", m.fromnative([0xff, 0x81], DEF, False))                      # -32257
print("FR4", m.fromnative([0xff, 0x81], LE, False))                       # -32257
print("FR5", m.fromnative([0xff, 0x81], LE | UNS, False))                 # 33279
print("FR6", m.fromnative([0xff, 0x81], DEF, True))                       # unsigned 65409
print("FR7", m.fromnative([], DEF, False))                                # 0
print("FR8", m.fromnative([0, 0, 0, 0, 0, 0, 0, 0x80, 0xff], DEF, False)) # INT64_MIN
print("FR9", m.fromnative([0xff, 0xff], DEF, False))                      # -1
print("FR10", m.fromnative([0xff, 0xff], DEF, True))                      # 65535

# ---- PyLongWriter ----
print("W1", m.writer(1, [0, 0, 512]))        # -2^69
print("W2", m.writer(0, [0, 433305513, 852086020, 807]))  # 10^30
try:
    m.writer(0, [])
except ValueError as e:
    print("W3", type(e).__name__, e)
print("W4", m.writer_bad())                  # 0
print("W5", m.writer_discard())              # None (no crash)

# ---- PyUnstable_Long ----
print("U1", m.unstable_long(42))             # (1, 42)
print("U2", m.unstable_long(0))              # (1, 0)
print("U3", m.unstable_long(10**30))         # (0, 0)
print("U4", m.unstable_long(-7))             # (1, -7)

# ---- GC family + conservation ----
m.gc_setup()
print("G1", m.gc_track())                    # call-shape ok
for _ in range(10000):
    m.mk_track()
n_before = m.count_live()
import gc
gc.collect()
print("G2", m.count_live(), n_before)        # 10000 live through collect
m.drop_live()
gc.collect()
print("G3", m.count_live())                  # 0 after drop + collect
print("DONE")
