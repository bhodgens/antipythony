# Cites: S-05-4 (row enum), anchor #11, S-01 repr rules
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import enum
names = ["Enum", "IntEnum", "StrEnum", "Flag", "IntFlag", "ReprEnum",
         "FlagBoundary", "EnumCheck", "auto", "unique", "member",
         "nonmember", "verify", "show_flag_values"]
print(len(names), all(hasattr(enum, n) for n in names))
class C(enum.IntEnum):
    A = 1
print(C.A, int(C.A) + 1, C.A == 1, repr(C.A), str(C.A), C.A.name, C.A.value)
class Col(enum.Enum):
    RED = 1
    BLUE = 2
print(repr(Col.RED), [c.name for c in Col], Col["BLUE"] is Col.BLUE,
      Col(1) is Col.RED, list(Col)[1].value)
class SC(enum.StrEnum):
    A = "a"
print(repr(SC.A), str(SC.A), SC.A == "a")
class IF(enum.IntFlag):
    A = 1
    B = 2
    C = 4
print(repr(IF.A | IF.C), repr(IF(0)), IF.A | IF.B == IF(3),
      IF.A in IF.C | IF.A, repr(~IF.A))
class F(enum.Flag):
    X = enum.auto()
    Y = enum.auto()
print(repr(F.X), repr(F.X | F.Y), F.Y.value)
try:
    class Dup(enum.Enum):
        A = 1
        B = 1
    enum.unique(Dup)
except ValueError as e:
    print("unique:", e)
try:
    @enum.verify(enum.unique)
    class Dup2(enum.Enum):
        A = 1
        B = 1
except ValueError as e:
    print("verify:", str(e)[:30])
print(enum.show_flag_values(3))
class ReprMix(int, enum.ReprEnum):
    X = 1
print(repr(ReprMix.X), str(ReprMix.X))
try:
    class Bad(enum.Enum):
        A = 1
    Bad.A = 2
except AttributeError as e:
    print("assign:", e)
print(hasattr(enum, "global_enum"),
      hasattr(enum.EnumCheck, "CONTINUOUS") or hasattr(enum.EnumCheck, "member"))
class WithDoc(enum.Enum):
    "doc"
    A = enum.auto()
print(WithDoc.A.value, WithDoc.__module__)
print(len(list(IF)), [i.name for i in IF][:3])
print(enum.FlagBoundary is not None, enum.nonmember(1) == 1)
class Mem(enum.Enum):
    M = enum.member("v")
print(Mem.M.value)
