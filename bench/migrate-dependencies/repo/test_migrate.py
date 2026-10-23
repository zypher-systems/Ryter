import json
import tempfile
import unittest
from pathlib import Path

from migrate import Migration, Registry, Runner


def make(names, log):
    registry = Registry()
    for name in names:
        registry.register(Migration(name, up=lambda n=name: log.append(f'up {n}'),
                                    down=lambda n=name: log.append(f'down {n}')))
    return registry


class Apply(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.state = Path(self.dir.name) / 'state.json'
        self.log = []

    def tearDown(self):
        self.dir.cleanup()

    def test_applies_what_is_not_yet_applied_and_records_it(self):
        registry = make(['a', 'b'], self.log)
        runner = Runner(self.state)
        self.assertEqual(runner.apply(registry), ['a', 'b'])
        self.assertEqual(runner.apply(registry), [])
        self.assertEqual(self.log, ['up a', 'up b'])
        self.assertEqual(json.loads(self.state.read_text()), {'applied': ['a', 'b']})

    def test_dependencies_come_first(self):
        registry = Registry()
        registry.register(Migration('b', up=lambda: self.log.append('b'), depends_on=['a']))
        registry.register(Migration('a', up=lambda: self.log.append('a')))
        self.assertEqual([m.name for m in registry.order()], ['a', 'b'])

    def test_rollback_undoes_the_last_one(self):
        registry = make(['a', 'b'], self.log)
        runner = Runner(self.state)
        runner.apply(registry)
        self.assertEqual(runner.rollback(registry), ['b'])
        self.assertEqual(self.log[-1], 'down b')
        self.assertEqual(runner.applied(), ['a'])


if __name__ == '__main__':
    unittest.main()
