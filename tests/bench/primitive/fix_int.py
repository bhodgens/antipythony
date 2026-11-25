"""Differential probe: int.to_bytes / int.from_bytes / hex / oct / bin vs CPython 3.14.

Run under BOTH /opt/homebrew/bin/python3.14 and the antipythony binary:
    <interp> tests/bench/primitive/fix_int.py
Both runs must print identical lines (byte-exact gate, ppfix-int).

No `random` module use (ours lacks random.Random — filed P08 blocker);
a deterministic LCG supplies the pseudo-random bigint sweep instead.
"""
out = []
p = lambda x: out.append(str(x))

# deterministic pseudo-random bits (xorshift64), pure Python
seed = 0x243F6A8885A308D3
def nextbits(n):
    global seed
    seed ^= (seed << 13) & 0xFFFFFFFFFFFFFFFF
    seed ^= seed >> 7
    seed ^= (seed << 17) & 0xFFFFFFFFFFFFFFFF
    v = seed >> (64 - n) if n <= 64 else ((seed << 64) | (seed ^ 0xA5A5A5A5)) % (1 << n)
    return v if seed & 1 else -v

# ---- to_bytes: values 0/1/-1/max/min/2^63/2^64/2^70 + pseudo-random bigints
values = [0, 1, -1, 255, 2**63, 2**64, 2**70, -(2**63), -(2**70),
          2**63 - 1, -(2**63 - 1), 2**8 - 1, 2**16 - 1, 2**31, 2**128, -(2**128)]
values += [nextbits(1 + (i * 37) % 320) for i in range(40)]
for v in values:
    for ln in range(0, 41):
        for order in ("big", "little"):
            for signed in (True, False):
                try:
                    got = v.to_bytes(ln, order, signed=signed)
                except OverflowError as e:
                    p(f"OE {v} {ln} {order} {signed} {e}")
                except ValueError as e:
                    p(f"VE {v} {ln} {order} {signed} {e}")
                else:
                    p(f"OK {v} {ln} {order} {signed} {got.hex()}")
                    back = int.from_bytes(got, order, signed=signed)
                    assert back == v, (v, ln, order, signed, back)

# ---- from_bytes: 1..64-byte inputs
for n in range(1, 65):
    data = bytes((nextbits(8) & 0xFF) if False else (i * 131 + n * 17 + (seed >> (n % 40)) % 256) % 256 for i in range(n))
    for order in ("big", "little"):
        for signed in (True, False):
            p(f"FB {n} {order} {signed} {int.from_bytes(data, order, signed=signed)}")

# exact value spot checks (incl. the audit repros)
p("V1 " + str(int.from_bytes(bytes(range(32)), 'big')))
p("V2 " + (1 << 70).to_bytes(16, 'big').hex())
p("V3 " + (123456789012345678901234567890).to_bytes(16, 'little').hex())
p("V4 " + str(int.from_bytes(b'\xff' * 9, 'big', signed=True)))
p("V5 " + str(int.from_bytes(b'', 'big')))
p("V6 " + str((0).to_bytes(0, 'big')))
p("V7 " + str((0).to_bytes(0, 'big', signed=True)))
p("V8 " + (-1).to_bytes(8, 'little', signed=True).hex())
p("V9 " + (-2**63).to_bytes(8, 'big', signed=True).hex())

# error texts (CPython-exact)
def err(fn):
    try:
        fn()
        return "NO-ERROR"
    except Exception as e:
        return f"{type(e).__name__}: {e}"
p("E1 " + err(lambda: (256).to_bytes(1, 'big')))
p("E2 " + err(lambda: (-1).to_bytes(1, 'big')))
p("E3 " + err(lambda: (128).to_bytes(1, 'big', signed=True)))
p("E4 " + err(lambda: (-129).to_bytes(1, 'big', signed=True)))
p("E5 " + err(lambda: (1).to_bytes(-1, 'big')))
p("E6 " + err(lambda: (1).to_bytes(1, 'middle')))
p("E7 " + err(lambda: int.from_bytes(b'\x01', 'middle')))
p("E8 " + err(lambda: (127).to_bytes(1, 'big', signed=True)))
p("E9 " + err(lambda: (-128).to_bytes(1, 'big', signed=True)))
p("E10 " + err(lambda: (-1).to_bytes(0, 'big', signed=True)))
p("E11 " + err(lambda: (1).to_bytes(0, 'big', signed=True)))

# ---- hex / oct / bin (small, medium, huge; negatives; both sign variants)
for v in (0, 1, -1, 255, 2**64, 10**100, 255 * 256**100, 2**129 - 1):
    for fn in (hex, oct, bin):
        p("B " + fn(v))
        p("B " + fn(-v))

print("\n".join(out))
