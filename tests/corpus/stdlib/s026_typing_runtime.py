# Cites: S-05-4 (row typing), S-01 repr byte rules
# Slice: stdlib-v1 (T-11, gates 02.08). Reference bytes pinned by
# /opt/homebrew/bin/python3.14 (3.14.7; provisional vs the C-01 pin 3.14.0,
# per the T-05/T-06 provenance-gap precedent).
import collections.abc
import typing
names = ["Any", "Optional", "Union", "Literal", "ClassVar", "Final",
         "Annotated", "TypeVar", "Generic", "Protocol", "runtime_checkable",
         "NamedTuple", "TypedDict", "Callable", "Type", "Self", "Never",
         "NoReturn", "cast", "overload", "get_type_hints", "get_origin",
         "get_args", "TYPE_CHECKING", "ForwardRef", "TypeAlias",
         "TypeAliasType", "NewType", "ParamSpec", "TypeVarTuple", "Unpack",
         "Required", "NotRequired", "TypeGuard", "TypeIs",
         "dataclass_transform", "assert_never", "assert_type",
         "is_protocol", "is_typeddict", "get_protocol_members",
         "reveal_type"]
print(len(names), all(hasattr(typing, n) for n in names))
print(repr(typing.Union[int, str]), repr(typing.Optional[int]),
      typing.Optional[int] == typing.Union[int, None])
print(typing.get_origin(typing.Optional[int]) is typing.Union,
      typing.get_args(typing.Optional[int]))
print(typing.get_origin(int | str) is typing.Union,
      typing.get_args(typing.Literal["a", 1]))
print(str(typing.List[int]), str(typing.Any), str(typing.NoReturn))
print(repr(typing.Annotated[int, "meta"]),
      typing.get_args(typing.Annotated[int, "meta"]))
print(typing.TYPE_CHECKING, typing.cast(list, [1]) == [1])
NT = typing.NamedTuple("NT", [("n", int), ("s", str)])
print(NT(1, "a"), NT._fields, typing.get_type_hints(NT)["n"] is int)
class TD(typing.TypedDict):
    k: int
print(typing.is_typeddict(TD), list(TD.__annotations__))
@typing.runtime_checkable
class Proto(typing.Protocol):
    def foo(self): ...
print(isinstance(type("X", (object,), {"foo": lambda s: 1})(), Proto),
      sorted(typing.get_protocol_members(Proto)))
print(typing.is_protocol(Proto))
UserId = typing.NewType("UserId", int)
print(UserId(3) == 3, UserId.__name__)
T = typing.TypeVar("T")
class Box(typing.Generic[T]):
    def __init__(self, v):
        self.v = v
print(typing.get_origin(Box[int]) is Box, typing.get_args(Box[int]))
TA = typing.TypeAliasType("TA", int)
print(TA.__name__, TA.__value__ is int)
def fn(a: "int") -> "str":
    return str(a)
print(typing.get_type_hints(fn))
print(typing.get_args(typing.Callable[[int], str]),
      typing.get_origin(typing.Callable[[int], str]) is collections.abc.Callable)
print(str(typing.Final[int]), typing.Never is typing.NoReturn)
print(typing.assert_type(1, int) == 1)
try:
    typing.assert_never(1)
except (TypeError, AssertionError) as e:
    print("assert_never runtime:", type(e).__name__)
print(typing.get_overloads is not None, str(typing.Text),
      typing.ForwardRef("X") is not None)
