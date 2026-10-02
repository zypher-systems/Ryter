import unittest
from decimal import Decimal

import ledger
from ledger import Ledger


class HiddenLedgerTest(unittest.TestCase):
    def test_total_converts_through_eur(self):
        book = Ledger()
        book.add("10", "EUR")
        book.add("25", "USD")
        book.add("8", "GBP")
        self.assertEqual(book.total("EUR"), Decimal("40.00"))
        self.assertEqual(book.total("USD"), Decimal("50.00"))
        self.assertEqual(book.total("JPY"), Decimal("6400.00"))

    def test_the_sum_is_rounded_once(self):
        # 0.01 USD is 0.008 EUR. Rounded one at a time, three of them are
        # 0.03 EUR; added up first they are 0.024, which is 0.02.
        book = Ledger()
        for _ in range(3):
            book.add("0.01", "USD")
        self.assertEqual(book.total("EUR"), Decimal("0.02"))

    def test_half_goes_to_even(self):
        book = Ledger(rates={"EUR": Decimal("1"), "XTS": Decimal("1")})
        book.add("0.125", "XTS")
        self.assertEqual(book.total("EUR"), Decimal("0.12"))
        book.add("0.01", "XTS")
        self.assertEqual(book.total("EUR"), Decimal("0.14"))

    def test_an_empty_ledger_totals_zero_with_two_places(self):
        total = Ledger().total("EUR")
        self.assertEqual(total, Decimal("0.00"))
        self.assertEqual(str(total), "0.00")

    def test_an_unknown_target_currency(self):
        book = Ledger()
        book.add("1", "EUR")
        with self.assertRaises(ledger.UnknownCurrency) as caught:
            book.total("XXX")
        self.assertIsInstance(caught.exception, LookupError)
        self.assertEqual(caught.exception.code, "XXX")
        self.assertIn("XXX", str(caught.exception))

    def test_add_refuses_an_unknown_currency_and_adds_nothing(self):
        book = Ledger()
        book.add("5", "EUR")
        with self.assertRaises(ledger.UnknownCurrency):
            book.add("1", "XXX")
        self.assertEqual(len(book.entries), 1)
        self.assertEqual(book.total("EUR"), Decimal("5.00"))

    def test_balance_is_unchanged(self):
        book = Ledger()
        book.add("10.50", "USD")
        book.add("7", "EUR")
        self.assertEqual(book.balance("USD"), Decimal("10.50"))
        self.assertEqual(book.balance("GBP"), Decimal("0"))

    def test_the_error_lives_in_its_own_module(self):
        from ledger.errors import UnknownCurrency

        self.assertIs(UnknownCurrency, ledger.UnknownCurrency)


if __name__ == "__main__":
    unittest.main()
