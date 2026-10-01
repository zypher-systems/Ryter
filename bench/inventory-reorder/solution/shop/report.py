def stock_report(inventory):
    """One line per item, by sku: "<sku> <name>: <stock>"."""
    return [f"{i.sku} {i.name}: {i.stock}" for i in inventory.items()]


def reorder_report(inventory):
    """One line per item that needs reordering, by sku."""
    return [
        f"{i.sku} {i.name}: stock {i.stock}, order {i.reorder_qty}"
        for i in inventory.needs_reorder()
    ]
