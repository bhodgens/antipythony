type Vector = list[float]
type Pair[T] = tuple[T, T]
type Recursive[T] = T | list[Recursive[T]]

def first[P, *Ts, **Kw](p: P, *ts: Ts, **kw: Kw) -> P:
    return p
