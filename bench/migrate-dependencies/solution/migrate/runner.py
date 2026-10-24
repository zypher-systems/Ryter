import json
import os
import tempfile
from pathlib import Path

from .errors import IrreversibleMigration, MigrationFailed


class Runner:
    """Applies a registry's migrations and remembers which ran."""

    def __init__(self, state_path):
        self.state_path = Path(state_path)

    def applied(self):
        if not self.state_path.exists():
            return []
        data = json.loads(self.state_path.read_text())
        if isinstance(data, list):
            return list(data)
        return list(data.get('applied', []))

    def _save(self, applied):
        fd, tmp = tempfile.mkstemp(prefix=self.state_path.name + '.', dir=self.state_path.parent)
        try:
            with os.fdopen(fd, 'w') as stream:
                json.dump({'applied': list(applied)}, stream)
            os.replace(tmp, self.state_path)
        except BaseException:
            os.unlink(tmp)
            raise

    def apply(self, registry, dry_run=False):
        applied = self.applied()
        pending = [m for m in registry.order() if m.name not in applied]
        if dry_run:
            return [m.name for m in pending]
        ran = []
        for migration in pending:
            try:
                migration.up()
            except Exception as e:
                raise MigrationFailed(migration.name) from e
            applied.append(migration.name)
            ran.append(migration.name)
            self._save(applied)
        return ran

    def rollback(self, registry, count=1):
        applied = self.applied()
        targets = list(reversed(applied[-count:])) if count > 0 else []
        for name in targets:
            if registry.get(name).down is None:
                raise IrreversibleMigration(name)
        undone = []
        for name in targets:
            registry.get(name).down()
            applied.remove(name)
            undone.append(name)
            self._save(applied)
        return undone
