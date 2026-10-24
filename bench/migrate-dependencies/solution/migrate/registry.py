import heapq

from .errors import CycleError, UnknownDependency


class Registry:
    """The migrations known, ordered by what they depend on."""

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
        for m in self._migrations.values():
            for dep in m.depends_on:
                if dep not in self._migrations:
                    raise UnknownDependency(m.name, dep)
        waiting = {name: set(m.depends_on) for name, m in self._migrations.items()}
        dependents = {name: [] for name in self._migrations}
        for name, deps in waiting.items():
            for dep in deps:
                dependents[dep].append(name)
        ready = [name for name, deps in waiting.items() if not deps]
        heapq.heapify(ready)
        placed = []
        while ready:
            name = heapq.heappop(ready)
            placed.append(self._migrations[name])
            for other in dependents[name]:
                waiting[other].discard(name)
                if not waiting[other]:
                    heapq.heappush(ready, other)
        if len(placed) < len(self._migrations):
            raise CycleError(self._cycle({name for name, deps in waiting.items() if deps}))
        return placed

    def _cycle(self, stuck):
        """The names on a cycle among `stuck`, in dependency order, from the
        smallest name that lies on one."""
        for start in sorted(stuck):
            path, seen = [], {}
            node = start
            while node not in seen:
                seen[node] = len(path)
                path.append(node)
                # Follow an unplaced dependency; prefer one that leads back.
                node = min(d for d in self._migrations[node].depends_on if d in stuck)
            cycle = path[seen[node]:]
            if start in cycle:
                i = cycle.index(start)
                return cycle[i:] + cycle[:i]
        raise AssertionError('no cycle found among stuck migrations')
