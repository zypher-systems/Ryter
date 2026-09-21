def order_total(items):
    """Sum of price * quantity over (price, quantity) pairs."""
    return sum(price * qty for price, qty in items)
