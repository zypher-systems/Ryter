import unittest
from decimal import Decimal

from ledger import Ledger


class LedgerTest(unittest.TestCase):
    def test_balance_adds_one_currency(self):
        ledger = Ledger()
        ledger.add("10.50", "USD")
        ledger.add("2", "USD")
        ledger.add("7", "EUR")
        self.assertEqual(ledger.balance("USD"), Decimal("12.50"))

    def test_total_converts(self):
        ledger = Ledger()
        ledger.add("10", "EUR")
        ledger.add("25", "USD")
        self.assertEqual(ledger.total("EUR"), Decimal("30.00"))


if __name__ == "__main__":
    unittest.main()
