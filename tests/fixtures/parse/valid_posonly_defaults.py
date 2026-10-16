def clamp(v, lo=0, hi=10, /):
    return max(lo, min(hi, v))

def kinds(*, a, b="b"):
    return a + b
