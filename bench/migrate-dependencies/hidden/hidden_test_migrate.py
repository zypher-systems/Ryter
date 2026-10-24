import json
import os
import tempfile
import unittest
from pathlib import Path

from migrate import Migration, Registry, Runner
from migrate.errors import CycleError, IrreversibleMigration, MigrateError, MigrationFailed, UnknownDependency


def make(spec, log, down=True):
    """spec: {name: [dependencies]}."""
    registry = Registry()
    for name, deps in spec.items():
        registry.register(Migration(name, up=lambda n=name: log.append(f'up {n}'),
                                    down=(lambda n=name: log.append(f'down {n}')) if down else None,
                                    depends_on=deps))
    return registry


class Order(unittest.TestCase):
    def names(self, spec):
        return [m.name for m in make(spec, []).order()]

    def test_dependencies_first_then_the_smallest_name(self):
        self.assertEqual(self.names({'c': [], 'b': ['c'], 'a': ['b']}), ['c', 'b', 'a'])
        self.assertEqual(self.names({'z': [], 'y': [], 'x': ['z']}), ['y', 'z', 'x'])
        self.assertEqual(self.names({'d': ['b', 'c'], 'c': ['a'], 'b': ['a'], 'a': []}), ['a', 'b', 'c', 'd'])
        self.assertEqual(self.names({'m2': ['m10'], 'm10': [], 'm1': []}), ['m1', 'm10', 'm2'])

    def test_the_order_does_not_depend_on_registration_order(self):
        a = self.names({'b': ['a'], 'a': [], 'c': ['a']})
        b = self.names({'c': ['a'], 'a': [], 'b': ['a']})
        self.assertEqual(a, b)
        self.assertEqual(a, ['a', 'b', 'c'])

    def test_an_unknown_dependency(self):
        with self.assertRaises(UnknownDependency) as caught:
            make({'a': ['zzz']}, []).order()
        self.assertEqual((caught.exception.name, caught.exception.dependency), ('a', 'zzz'))
        self.assertIsInstance(caught.exception, MigrateError)

    def test_a_cycle_names_its_members_from_the_smallest(self):
        with self.assertRaises(CycleError) as caught:
            make({'c': ['b'], 'b': ['a'], 'a': ['c'], 'x': []}, []).order()
        self.assertEqual(caught.exception.names, ['a', 'c', 'b'])
        with self.assertRaises(CycleError) as caught:
            make({'a': ['a']}, []).order()
        self.assertEqual(caught.exception.names, ['a'])
        with self.assertRaises(CycleError) as caught:
            make({'q': ['p'], 'p': ['q'], 'z': ['q']}, []).order()
        self.assertEqual(caught.exception.names, ['p', 'q'])


class Runs(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.state = Path(self.dir.name) / 'state.json'
        self.log = []

    def tearDown(self):
        self.dir.cleanup()

    def saved(self):
        return json.loads(self.state.read_text())

    def test_dry_run_plans_and_touches_nothing(self):
        registry = make({'b': ['a'], 'a': []}, self.log)
        runner = Runner(self.state)
        self.assertEqual(runner.apply(registry, dry_run=True), ['a', 'b'])
        self.assertEqual(self.log, [])
        self.assertFalse(self.state.exists())
        runner.apply(registry)
        self.assertEqual(runner.apply(registry, dry_run=True), [])

    def test_a_failure_keeps_what_ran_before_it(self):
        registry = Registry()
        registry.register(Migration('a', up=lambda: self.log.append('a')))
        registry.register(Migration('b', up=lambda: self.log.append('b'), depends_on=['a']))

        def boom():
            raise RuntimeError('disk full')
        registry.register(Migration('c', up=boom, depends_on=['b']))
        registry.register(Migration('d', up=lambda: self.log.append('d'), depends_on=['c']))
        runner = Runner(self.state)
        with self.assertRaises(MigrationFailed) as caught:
            runner.apply(registry)
        self.assertEqual(caught.exception.name, 'c')
        self.assertIsInstance(caught.exception.__cause__, RuntimeError)
        self.assertEqual(str(caught.exception.__cause__), 'disk full')
        self.assertEqual(self.log, ['a', 'b'])
        self.assertEqual(self.saved(), {'applied': ['a', 'b']})

    def test_the_state_is_written_after_each_step(self):
        seen = []
        registry = Registry()
        registry.register(Migration('a', up=lambda: seen.append(self.saved() if self.state.exists() else None)))
        registry.register(Migration('b', up=lambda: seen.append(self.saved()), depends_on=['a']))
        Runner(self.state).apply(registry)
        self.assertEqual(seen, [None, {'applied': ['a']}])

    def test_rollback_checks_every_down_first(self):
        registry = Registry()
        registry.register(Migration('a', up=lambda: None, down=lambda: self.log.append('down a')))
        registry.register(Migration('b', up=lambda: None, down=None, depends_on=['a']))
        registry.register(Migration('c', up=lambda: None, down=lambda: self.log.append('down c'), depends_on=['b']))
        runner = Runner(self.state)
        runner.apply(registry)
        with self.assertRaises(IrreversibleMigration) as caught:
            runner.rollback(registry, count=3)
        self.assertEqual(caught.exception.name, 'b')
        self.assertEqual(self.log, [])
        self.assertEqual(self.saved(), {'applied': ['a', 'b', 'c']})
        self.assertEqual(runner.rollback(registry, count=1), ['c'])
        self.assertEqual(self.log, ['down c'])

    def test_rollback_records_each_step_and_takes_more_than_applied(self):
        registry = make({'a': [], 'b': ['a'], 'c': ['b']}, self.log)
        runner = Runner(self.state)
        runner.apply(registry)
        seen = []
        registry.get('a').down = lambda: seen.append(self.saved())
        self.assertEqual(runner.rollback(registry, count=10), ['c', 'b', 'a'])
        self.assertEqual(seen, [{'applied': ['a']}])
        self.assertEqual(self.saved(), {'applied': []})
        self.assertEqual(runner.rollback(registry), [])

    def test_the_old_state_format_still_loads(self):
        self.state.write_text(json.dumps(['a']))
        registry = make({'a': [], 'b': ['a']}, self.log)
        runner = Runner(self.state)
        self.assertEqual(runner.applied(), ['a'])
        self.assertEqual(runner.apply(registry), ['b'])
        self.assertEqual(self.log, ['up b'])
        self.assertEqual(self.saved(), {'applied': ['a', 'b']})

    def test_writes_replace_the_file_by_renaming(self):
        registry = make({'a': []}, self.log)
        runner = Runner(self.state)
        runner.apply(registry)
        before = os.stat(self.state).st_ino
        registry.register(Migration('b', up=lambda: None))
        runner.apply(registry)
        self.assertNotEqual(os.stat(self.state).st_ino, before, 'the file was rewritten in place')
        self.assertEqual(sorted(p.name for p in Path(self.dir.name).iterdir()), ['state.json'])

    def test_an_unknown_dependency_stops_apply_before_anything_runs(self):
        registry = make({'a': ['nope']}, self.log)
        runner = Runner(self.state)
        with self.assertRaises(UnknownDependency):
            runner.apply(registry)
        self.assertEqual(self.log, [])
        self.assertFalse(self.state.exists())


if __name__ == '__main__':
    unittest.main()
