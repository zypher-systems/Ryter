"""A small config reader: `[section]` headers and `key = value` lines."""


class ConfigError(Exception):
    pass


class Config:
    def __init__(self, sections):
        self._sections = sections

    @classmethod
    def load(cls, text):
        sections = {}
        current = 'default'
        for number, raw in enumerate(text.splitlines(), 1):
            line = raw.strip()
            if not line or line[0] in '#;':
                continue
            if line.startswith('[') and line.endswith(']'):
                current = line[1:-1].strip()
                sections.setdefault(current, {})
                continue
            if '=' not in line:
                raise ConfigError(f'line {number}: expected key = value')
            key, _, value = line.partition('=')
            sections.setdefault(current, {})[key.strip()] = value.strip()
        return cls(sections)

    def sections(self):
        return sorted(self._sections)

    def raw(self, section, key):
        try:
            return self._sections[section][key]
        except KeyError:
            raise KeyError(f'{section}.{key}') from None

    def get(self, section, key):
        """The value as written; references like ${other.key} are not resolved yet."""
        return self.raw(section, key)
