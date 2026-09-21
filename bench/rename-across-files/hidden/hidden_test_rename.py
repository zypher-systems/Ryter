import pathlib
import unittest

import orders
from report import summary


class TestRenameHidden(unittest.TestCase):
    def test_new_name_exists(self):
        self.assertEqual(orders.order_total([(2, 3)]), 6)

    def test_old_name_is_gone(self):
        self.assertFalse(hasattr(orders, "calc_total"))
        for path in pathlib.Path(".").glob("*.py"):
            if path.name.startswith("hidden_"):
                continue
            self.assertNotIn("calc_total", path.read_text(), path.name)

    def test_behaviour_unchanged(self):
        self.assertEqual(summary([]), "total: 0")


if __name__ == "__main__":
    unittest.main()
