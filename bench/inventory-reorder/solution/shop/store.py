from shop.errors import InsufficientStock
from shop.models import Item


class Inventory:
    def __init__(self):
        self._items = {}

    def add_item(self, item: Item) -> None:
        if item.sku in self._items:
            raise KeyError(f"duplicate sku {item.sku}")
        self._items[item.sku] = item

    def get(self, sku: str) -> Item:
        return self._items[sku]

    def items(self):
        return sorted(self._items.values(), key=lambda i: i.sku)

    def add_stock(self, sku: str, qty: int) -> None:
        if qty <= 0:
            raise ValueError("qty must be positive")
        self._items[sku].stock += qty

    def remove_stock(self, sku: str, qty: int) -> None:
        if qty <= 0:
            raise ValueError("qty must be positive")
        item = self._items[sku]
        if qty > item.stock:
            raise InsufficientStock(sku, qty, item.stock)
        item.stock -= qty

    def needs_reorder(self):
        return [i for i in self.items() if i.reorder_level > 0 and i.stock <= i.reorder_level]
