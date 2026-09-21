import re
import unicodedata


def title_case(text):
    """Capitalise each word."""
    return " ".join(w.capitalize() for w in text.split())


def slugify(text):
    """Lowercase ASCII words joined by single hyphens."""
    ascii_text = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    return re.sub(r"[^a-z0-9]+", "-", ascii_text.lower()).strip("-")
