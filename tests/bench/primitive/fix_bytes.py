"""Differential probe: bytes find/rfind/index/rindex/count/replace/split/rsplit
vs CPython 3.14 (fix_bytes, leaf pp2-memchr).

Run under BOTH /opt/homebrew/bin/python3.14 and the antipythony binary:
    <interp> tests/bench/primitive/fix_bytes.py
Both runs must print identical lines (byte-exact gate).

Covers: needle lengths 0/1/2/8/64, int needles, hay with/without matches,
overlapping matches (count/replace semantics), start/end bounds, maxsplit,
empty-needle rules, error types + texts.
"""
out = []
p = lambda x: out.append(str(x))

def err(fn):
    try:
        fn()
        return "NO-ERROR"
    except Exception as e:
        return f"{type(e).__name__}: {e}"

HAY = b"The quick brown fox jumps over the lazy dog. 0123456789 abcabcabc END"
NEEDLES = [b"", b"x", b"oo", b"brown fox", b"a" * 64, b"abcabc", b"END", b"Z"]
INT_NEEDLES = [98, -100, 300, 0, 255]
REPS = [b"", b"-", b"XY", b"ab" * 30]
SEPS = [b" ", b"o", b"abc", b"XY" * 20, b"~"]

# ---- find / rfind / index / rindex, full + bounded, with/without matches
for nd in NEEDLES:
    for meth in ("find", "rfind", "index", "rindex"):
        try:
            p(f"{meth}:{nd!r}:{getattr(HAY, meth)(nd)}")
        except ValueError as e:
            p(f"{meth}:{nd!r}:VE:{e}")
        for st in (-100, -1, 0, 1, 10, 44, 70, 100):
            for en in (-100, -1, 0, 9, 44, 70, 71, 200, None):
                args = (st,) if en is None else (st, en)
                try:
                    r = getattr(HAY, meth)(*args)
                    p(f"B {meth} {nd!r} {args} {r}")
                except ValueError as e:
                    p(f"B {meth} {nd!r} {args} VE:{e}")

# ---- int-needle semantics (CPython: int = single byte; range-checked)
for iv in INT_NEEDLES:
    for meth in ("find", "rfind", "index", "rindex", "count"):
        try:
            p(f"I {meth} {iv} {getattr(HAY, meth)(iv)}")
        except ValueError as e:
            p(f"I {meth} {iv} VE:{e}")
        except TypeError as e:
            p(f"I {meth} {iv} TE:{e}")
        except Exception as e:
            p(f"I {meth} {iv} XX:{type(e).__name__}:{e}")
    try:
        p(f"IR {iv} {HAY.replace(iv, b'-')!r}")
    except (TypeError, ValueError) as e:
        p(f"IR {iv} {type(e).__name__}:{e}")
    try:
        p(f"IS {iv} {HAY.startswith(iv)}")
    except TypeError as e:
        p(f"IS {iv} TE:{e}")

# ---- type errors (str/float needles)
p("TE1 " + err(lambda: HAY.find("x")))
p("TE2 " + err(lambda: HAY.find(3.5)))
p("TE3 " + err(lambda: HAY.count("x")))
p("TE4 " + err(lambda: HAY.replace("x", b"-")))
p("TE5 " + err(lambda: HAY.replace(b"o", "-")))
p("TE6 " + err(lambda: HAY.split("o")))

# ---- empty-needle find semantics
for meth in ("find", "rfind", "index", "rindex"):
    for st in (0, 3, 100, -100):
        try:
            p(f"EF {meth} {st} {HAY.__getattribute__(meth)(b'', st)}")
        except ValueError as e:
            p(f"EF {meth} {st} VE:{e}")

# ---- count: overlapping + bounded
c1 = b"aaaa" + b"abab" + b"aa"
for nd in (b"aa", b"abab", b"aba", b"", b"zz"):
    p(f"C plain {nd!r} {c1.count(nd)}")
    for st, en in ((0, None), (1, None), (0, 7), (2, 8), (-4, None), (0, -5), (3, 3), (5, 5), (7, 2)):
        args = (st,) if en is None else (st, en)
        try:
            p(f"CB {nd!r} {args} {c1.count(*args)}")
        except ValueError as e:
            p(f"CB {nd!r} {args} VE:{e}")

# ---- count/replace on 48KiB-ish repetitive hay (overlap semantics at scale)
big = (b"abcabcX" * 7000)
p(f"CBIG {big.count(b'abcabc')}")
p(f"RBIG {len(big.replace(b'abcabc', b'Q'))} {big.replace(b'abcabc', b'Q').count(b'Q')}")

# ---- replace: plain, overlapping, count-limited, empty old/new
for old in NEEDLES:
    for new in REPS:
        p(f"R {old!r} {new!r} {HAY.replace(old, new)!r}")
for old, cnt in ((b"o", 0), (b"o", 1), (b"o", 2), (b"o", -1), (b"o", 10**9),
                 (b"", 0), (b"", 1), (b"", 2), (b"", 5), (b"", -1), (b"abcabc", 1), (b"abcabc", 2)):
    p(f"RC {old!r} {cnt} {HAY.replace(old, b'-', cnt)!r}")

# ---- replace error texts
p("RE1 " + err(lambda: HAY.replace(1, b"-")))
p("RE2 " + err(lambda: HAY.replace(b"o", 1)))

# ---- split / rsplit: sep + maxsplit + whitespace arm
for sep in SEPS:
    for ms in (-1, 0, 1, 2, 100):
        try:
            p(f"S {sep!r} {ms} {HAY.split(sep, ms)!r}")
            p(f"RS {sep!r} {ms} {HAY.rsplit(sep, ms)!r}")
        except ValueError as e:
            p(f"SE {sep!r} {ms} VE:{e}")
ws = b"  lead\tmid\tdouble  trail \n mix \r\n end  "
for ms in (-1, 0, 1, 3, 100):
    p(f"WS {ms} {ws.split(None, ms)!r}")
    p(f"WRS {ms} {ws.rsplit(None, ms)!r}")
p("WSnone " + repr(ws.split()))
p("WSempty " + repr(b"   ".split()) + repr(b"   ".rsplit()) + repr(b"".split()) + repr(b"".rsplit()))
edge = b"a" + b" " * 4 + b"b"
p(f"WSE {edge.split(None, 1)!r} {edge.rsplit(None, 1)!r} {edge.split(None, 0)!r} {edge.rsplit(None, 0)!r}")

# ---- split error texts
p("SE1 " + err(lambda: HAY.split(b"")))
p("SE2 " + err(lambda: HAY.rsplit(b"", 2)))
p("SE3 " + err(lambda: HAY.split(1)))

# ---- index/rindex ValueError texts
p("IE1 " + err(lambda: HAY.index(b"Z")))
p("IE2 " + err(lambda: HAY.rindex(b"Z")))

print("\n".join(x for x in out if x))
