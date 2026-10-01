PREFIX = "APP_"


def parse(value):
    """A string from the environment as a bool, int, float or str."""
    if value.isdigit():
        return int(value)
    return bool(value) if value in ("true", "false") else value


def overrides(environ):
    """The `APP_` variables as a nested dictionary of overrides."""
    out = {}
    for name, value in environ.items():
        if not name.startswith(PREFIX):
            continue
        out[name[len(PREFIX):]] = parse(value)
    return out
