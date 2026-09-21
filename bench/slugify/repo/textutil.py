def title_case(text):
    """Capitalise each word."""
    return " ".join(w.capitalize() for w in text.split())
