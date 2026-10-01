import unittest

from shop.models import Item
from shop.report import stock_report
from shop.store import Inventory


class ShopTest(unittest.TestCase):
    def setUp(self):
        self.inv = Inventory()
        self.inv.add_item(Item("B2", "bolt", stock=10))
        self.inv.add_item(Item("A1", "anvil", stock=1))

    def test_stock_report_is_by_sku(self):
        self.assertEqual(stock_report(self.inv), ["A1 anvil: 1", "B2 bolt: 10"])

    def test_add_and_remove_stock(self):
        self.inv.add_stock("A1", 4)
        self.inv.remove_stock("A1", 2)
        self.assertEqual(self.inv.get("A1").stock, 3)

    def test_item_takes_reorder_fields(self):
        item = Item("C3", "chain", stock=2, reorder_level=5, reorder_qty=20)
        self.assertEqual((item.reorder_level, item.reorder_qty), (5, 20))

    def test_a_low_item_needs_reorder(self):
        self.inv.add_item(Item("C3", "chain", stock=2, reorder_level=5, reorder_qty=20))
        self.assertEqual([i.sku for i in self.inv.needs_reorder()], ["C3"])


if __name__ == "__main__":
    unittest.main()
