"""Settings: defaults, with overrides from the environment."""

from settings.env import overrides
from settings.merge import deep_merge


def load(defaults, environ):
    """`defaults` with the `APP_` variables of `environ` applied."""
    return deep_merge(defaults, overrides(environ))
