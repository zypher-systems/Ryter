class MigrateError(Exception):
    """Anything the runner or the registry refuses."""


class UnknownDependency(MigrateError):
    def __init__(self, name, dependency):
        super().__init__(f'{name} depends on {dependency}, which is not registered')
        self.name = name
        self.dependency = dependency


class CycleError(MigrateError):
    def __init__(self, names):
        super().__init__('dependency cycle: ' + ' -> '.join(names))
        self.names = list(names)


class MigrationFailed(MigrateError):
    def __init__(self, name):
        super().__init__(f'{name} failed')
        self.name = name


class IrreversibleMigration(MigrateError):
    def __init__(self, name):
        super().__init__(f'{name} cannot be undone')
        self.name = name
