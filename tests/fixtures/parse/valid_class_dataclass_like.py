from dataclasses import dataclass, field

@dataclass
class Item:
    name: str
    tags: list[str] = field(default_factory=list)
    count: int = 1
