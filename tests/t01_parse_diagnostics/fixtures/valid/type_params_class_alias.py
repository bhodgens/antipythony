# T-01 valid fixture: PEP 695 generic classes and `type` aliases (3.12+ grammar).
# Cites: C-03 (3.14 grammar); plan-01 Stmt list names classes — generic class header is part of the same def grammar.
from collections.abc import Callable
class Box[T]:
    def __init__(self, item: T):
        self.item = item
class Map[K, V]:
    pass
type Ints = list[int]
type Handler[T] = Callable[[T], T]
print(Box(1), Map(), Ints)
