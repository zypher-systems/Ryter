import unittest

from shop.errors import InsufficientStock
from shop.models import Item
from shop.report import reorder_report, stock_report
from shop.store import Inventory


class HiddenShopTest(unittest.TestCase):
    def setUp(self):
        self.inv = Inventory()
        self.inv.add_item(Item("D4", "drill", stock=3, reorder_level=3, reorder_qty=6))
        self.inv.add_item(Item("B2", "bolt", stock=0, reorder_level=10, reorder_qty=100))
        self.inv.add_item(Item("A1", "anvil", stock=0))
        self.inv.add_item(Item("C3", "chain", stock=9, reorder_level=5, reorder_qty=20))

    def test_defaults_are_zero(self):
        item = Item("Z9", "zip")
        self.assertEqual((item.stock, item.reorder_level, item.reorder_qty), (0, 0, 0))

    def test_negative_reorder_values_are_refused(self):
        with self.assertRaises(ValueError):
            Item("Z9", "zip", reorder_level=-1)
        with self.assertRaises(ValueError):
            Item("Z9", "zip", reorder_qty=-5)

    def test_at_the_level_counts_and_no_level_never_does(self):
        # D4 is exactly at its level; A1 is out of stock but has no level.
        self.assertEqual([i.sku for i in self.inv.needs_reorder()], ["B2", "D4"])

    def test_reorder_report_lines(self):
        self.assertEqual(
            reorder_report(self.inv),
            ["B2 bolt: stock 0, order 100", "D4 drill: stock 3, order 6"],
        )

    def test_selling_down_puts_an_item_on_the_report(self):
        self.inv.remove_stock("C3", 4)
        self.assertIn("C3 chain: stock 5, order 20", reorder_report(self.inv))

    def test_too_much_raises_and_changes_nothing(self):
        with self.assertRaises(InsufficientStock) as caught:
            self.inv.remove_stock("D4", 4)
        err = caught.exception
        self.assertEqual((err.sku, err.requested, err.available), ("D4", 4, 3))
        self.assertEqual(self.inv.get("D4").stock, 3)

    def test_removing_all_of_it_is_fine(self):
        self.inv.remove_stock("D4", 3)
        self.assertEqual(self.inv.get("D4").stock, 0)

    def test_existing_behaviour_holds(self):
        self.assertEqual(stock_report(self.inv)[0], "A1 anvil: 0")
        with self.assertRaises(ValueError):
            self.inv.remove_stock("D4", 0)
        with self.assertRaises(KeyError):
            self.inv.add_item(Item("D4", "again"))


if __name__ == "__main__":
    unittest.main()
