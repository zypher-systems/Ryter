def pad_right(text, width):
    """`text` left-aligned in `width` columns."""
    return text + " " * max(0, width - len(text))


def pad_left(text, width):
    """`text` right-aligned in `width` columns."""
    return " " * max(0, width - len(text)) + text
