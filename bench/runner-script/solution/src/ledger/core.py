import json
import os
from decimal import ROUND_HALF_EVEN, Decimal

from .errors import UnknownCurrency

RATES_FILE = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "build", "rates.json"
)

CENT = Decimal("0.01")


def load_rates(path=RATES_FILE):
    """Units of each currency that one EUR buys."""
    with open(path) as f:
        return {code: Decimal(rate) for code, rate in json.load(f).items()}


class Ledger:
    def __init__(self, rates=None):
        self.rates = rates if rates is not None else load_rates()
        self.entries = []

    def _rate(self, currency):
        try:
            return self.rates[currency]
        except KeyError:
            raise UnknownCurrency(currency) from None

    def add(self, amount, currency):
        self._rate(currency)
        self.entries.append((Decimal(str(amount)), currency))

    def balance(self, currency):
        """The entries in `currency`, added up. Nothing is converted."""
        return sum((a for a, c in self.entries if c == currency), Decimal("0"))

    def total(self, currency):
        """Every entry converted into `currency`, added up, rounded once."""
        target = self._rate(currency)
        total = sum(
            (amount / self._rate(code) * target for amount, code in self.entries),
            Decimal("0"),
        )
        return total.quantize(CENT, rounding=ROUND_HALF_EVEN)
