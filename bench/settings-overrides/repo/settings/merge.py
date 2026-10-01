def deep_merge(base, over):
    """`base` with `over` applied on top."""
    for key, value in over.items():
        base[key] = value
    return base
