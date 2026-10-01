from dataclasses import dataclass


@dataclass
class Item:
    sku: str
    name: str
    stock: int = 0
    reorder_level: int = 0
    reorder_qty: int = 0

    def __post_init__(self):
        if self.stock < 0:
            raise ValueError("stock can't be negative")
        if self.reorder_level < 0:
            raise ValueError("reorder_level can't be negative")
        if self.reorder_qty < 0:
            raise ValueError("reorder_qty can't be negative")
