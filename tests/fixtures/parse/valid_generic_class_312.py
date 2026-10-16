class Box[T]:
    def __init__(self, item: T) -> None:
        self.item = item

    def get(self) -> T:
        return self.item

class Registry[KT, VT](dict):
    def add(self, k: KT, v: VT) -> None:
        self[k] = v
