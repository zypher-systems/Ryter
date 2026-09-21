from orders import order_total


def summary(items):
    return f"total: {order_total(items)}"
