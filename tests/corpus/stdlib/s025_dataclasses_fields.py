# Cites: S-05-4 (row dataclasses), anchor #11
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import dataclasses
names = ["dataclass", "field", "Field", "fields", "asdict", "astuple",
         "make_dataclass", "replace", "is_dataclass", "InitVar",
         "FrozenInstanceError", "MISSING", "KW_ONLY"]
print(len(names), all(hasattr(dataclasses, n) for n in names))
@dataclasses.dataclass
class P:
    x: int
print(P(1))
@dataclasses.dataclass
class DC:
    a: int
    b: str = "x"
    c: list = dataclasses.field(default_factory=lambda: [1])
    _d: int = dataclasses.field(init=False, repr=False, default=9)
print(DC(2), DC(2) == DC(2, b="x"), dataclasses.asdict(DC(3)),
      dataclasses.astuple(DC(3)))
print([f.name for f in dataclasses.fields(DC)])
try:
    DC(1, 2, 3, 4)
except TypeError as e:
    print("arity:", str(e)[:42])
@dataclasses.dataclass(frozen=True)
class FP:
    x: int
try:
    FP(1).x = 2
except dataclasses.FrozenInstanceError as e:
    print(type(e).__name__ + ":", e)
print(issubclass(dataclasses.FrozenInstanceError, AttributeError))
@dataclasses.dataclass
class KWO:
    x: int
    _: dataclasses.KW_ONLY
    y: int
print(KWO(x=1, y=2))
try:
    KWO(1, 2)
except TypeError as e:
    print("kwonly:", "positional" in str(e))
P2 = dataclasses.make_dataclass("P2", ["x",
    ("y", int, dataclasses.field(default=1))])
print(P2(5), dataclasses.is_dataclass(P2), dataclasses.is_dataclass(P2(1, 2)))
@dataclasses.dataclass
class IC:
    x: int
    iv: dataclasses.InitVar[int]
    def __post_init__(self, iv):
        print("post", iv * 2)
print(IC(1, 2))
print(dataclasses.replace(DC(7, b="z"), a=8) == DC(8, b="z"))
@dataclasses.dataclass(order=True)
class Or:
    v: int
print(Or(1) < Or(2), sorted([Or(2), Or(1)]) == [Or(1), Or(2)])
print(dataclasses.MISSING is not None, DC.__dataclass_fields__["a"].name)
@dataclasses.dataclass(unsafe_hash=True)
class H:
    x: int
print(hash(H(1)) == hash(H(1)))
print(DC(1) != 5, isinstance(dataclasses.field(), dataclasses.Field))
