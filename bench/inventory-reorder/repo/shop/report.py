def stock_report(inventory):
    """One line per item, by sku: "<sku> <name>: <stock>"."""
    return [f"{i.sku} {i.name}: {i.stock}" for i in inventory.items()]
