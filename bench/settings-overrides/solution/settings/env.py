PREFIX = "APP_"


def parse(value):
    """A string from the environment as a bool, int, float or str."""
    lowered = value.lower()
    if lowered in ("true", "false"):
        return lowered == "true"
    for kind in (int, float):
        try:
            return kind(value)
        except ValueError:
            pass
    return value


def overrides(environ):
    """The `APP_` variables as a nested dictionary of overrides."""
    out = {}
    for name, value in environ.items():
        if not name.startswith(PREFIX):
            continue
        *parents, last = name[len(PREFIX):].lower().split("__")
        at = out
        for key in parents:
            at = at.setdefault(key, {})
        at[last] = parse(value)
    return out
