class Registry:
    """The migrations known, in the order they were registered."""

    def __init__(self):
        self._migrations = {}

    def register(self, migration):
        if migration.name in self._migrations:
            raise ValueError(f'{migration.name} is registered twice')
        self._migrations[migration.name] = migration
        return migration

    def get(self, name):
        return self._migrations[name]

    def order(self):
        return list(self._migrations.values())
