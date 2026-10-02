import json
import os
from decimal import Decimal

RATES_FILE = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "..", "..", "build", "rates.json"
)


def load_rates(path=RATES_FILE):
    """Units of each currency that one EUR buys."""
    with open(path) as f:
        return {code: Decimal(rate) for code, rate in json.load(f).items()}


class Ledger:
    def __init__(self, rates=None):
        self.rates = rates if rates is not None else load_rates()
        self.entries = []

    def add(self, amount, currency):
        self.entries.append((Decimal(str(amount)), currency))

    def balance(self, currency):
        """The entries in `currency`, added up. Nothing is converted."""
        return sum((a for a, c in self.entries if c == currency), Decimal("0"))
