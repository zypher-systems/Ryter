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
        """The value with its references resolved."""
        return self._resolve(section, key, [])

    def _resolve(self, section, key, chain):
        name = f'{section}.{key}'
        if name in chain:
            loop = chain[chain.index(name):] + [name]
            raise ConfigError('cycle: ' + ' -> '.join(loop))
        value = self.raw(section, key)
        chain = chain + [name]
        out = []
        i = 0
        while i < len(value):
            if value[i] != '$':
                out.append(value[i])
                i += 1
            elif value.startswith('$$', i):
                out.append('$')
                i += 2
            elif value.startswith('${', i):
                end = value.find('}', i + 2)
                if end < 0:
                    raise ConfigError(f'unterminated reference in {name}')
                out.append(self._lookup(section, value[i + 2:end], name, chain))
                i = end + 1
            else:
                out.append('$')
                i += 1
        return ''.join(out)

    def _lookup(self, section, target, origin, chain):
        if '.' in target:
            tsec, _, tkey = target.partition('.')
            if tkey in self._sections.get(tsec, {}):
                return self._resolve(tsec, tkey, chain)
        else:
            for tsec in (section, 'default'):
                if target in self._sections.get(tsec, {}):
                    return self._resolve(tsec, target, chain)
        raise ConfigError(f'unknown key {target} referenced from {origin}')
