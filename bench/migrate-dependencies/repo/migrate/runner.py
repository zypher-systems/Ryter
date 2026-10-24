import json
from pathlib import Path


class Runner:
    """Applies a registry's migrations and remembers which ran."""

    def __init__(self, state_path):
        self.state_path = Path(state_path)

    def applied(self):
        if not self.state_path.exists():
            return []
        return json.loads(self.state_path.read_text())

    def _save(self, applied):
        self.state_path.write_text(json.dumps(applied))

    def apply(self, registry):
        applied = self.applied()
        ran = []
        for migration in registry.order():
            if migration.name in applied:
                continue
            migration.up()
            applied.append(migration.name)
            ran.append(migration.name)
        self._save(applied)
        return ran

    def rollback(self, registry, count=1):
        applied = self.applied()
        undone = []
        for name in reversed(applied[-count:] if count else []):
            registry.get(name).down()
            applied.remove(name)
            undone.append(name)
        self._save(applied)
        return undone
