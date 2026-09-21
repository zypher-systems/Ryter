def paginate(items, page, size):
    """Return the items on 1-based `page`, with `size` items per page."""
    if page < 1 or size < 1:
        raise ValueError("page and size must be at least 1")
    start = (page - 1) * size
    return items[start:start + size]
