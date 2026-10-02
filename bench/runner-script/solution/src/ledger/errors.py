class UnknownCurrency(LookupError):
    """A currency there is no rate for."""

    def __init__(self, code):
        super().__init__(f"unknown currency: {code}")
        self.code = code
