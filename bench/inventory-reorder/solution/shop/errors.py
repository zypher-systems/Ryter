class InsufficientStock(Exception):
    def __init__(self, sku, requested, available):
        super().__init__(f"{sku}: asked for {requested}, {available} in stock")
        self.sku = sku
        self.requested = requested
        self.available = available
