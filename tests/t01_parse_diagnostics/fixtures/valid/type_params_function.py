# T-01 valid fixture: PEP 695 type parameters on functions (3.12+; inside the 3.14 pin).
# Cites: C-03 (v1 supports the 3.14 grammar; type params are 3.12-3.14 grammar — NOT non-3.14 syntax). Task-named: "type params 3.12+".
def first[T](items: list[T]) -> T:
    return items[0]
def convert[T, U = str](value: T) -> U:
    return value
async def take[T](v: T) -> T:
    return v
print(first([1, 2]))
