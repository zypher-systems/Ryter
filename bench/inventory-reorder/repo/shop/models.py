from dataclasses import dataclass


@dataclass
class Item:
    sku: str
    name: str
    stock: int = 0

    def __post_init__(self):
        if self.stock < 0:
            raise ValueError("stock can't be negative")
