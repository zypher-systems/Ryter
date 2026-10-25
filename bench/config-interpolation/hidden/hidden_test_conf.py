import unittest

from conf import Config, ConfigError

TEXT = """
name = app
root = /srv/${name}

[paths]
data = ${root}/data
logs = ${data}/logs
name = paths-own-name

[web]
title = ${name} on ${paths.data}
static = ${paths.logs}/../static
dollars = $5 and $ and $$5 and $${x}
empty =
blank = [${empty}]
dup = first
dup = second

[loop]
a = ${b}
b = ${loop.c}
c = ${a}
me = ${me}
into = ${loop.a}

[bad]
open = ${never
missing = ${nowhere.key}
bare = ${nothing}
"""


def failure(cfg, section, key):
    try:
        cfg.get(section, key)
    except ConfigError as e:
        return str(e)
    raise AssertionError(f'{section}.{key} resolved')


class Scope(unittest.TestCase):
    def setUp(self):
        self.cfg = Config.load(TEXT)

    def test_a_bare_reference_falls_back_to_default(self):
        self.assertEqual(self.cfg.get('paths', 'data'), '/srv/app/data')

    def test_a_referenced_value_resolves_in_its_own_section(self):
        # `title` asks for `${name}` in web, which falls back to default's `app`;
        # `${paths.data}` resolves `${root}` from default, through paths.
        self.assertEqual(self.cfg.get('web', 'title'), 'app on /srv/app/data')
        # paths.logs refers to paths.data by its bare name, in paths.
        self.assertEqual(self.cfg.get('web', 'static'), '/srv/app/data/logs/../static')

    def test_chains_and_several_references_in_one_value(self):
        self.assertEqual(self.cfg.get('paths', 'logs'), '/srv/app/data/logs')

    def test_dollars(self):
        self.assertEqual(self.cfg.get('web', 'dollars'), '$5 and $ and $5 and ${x}')

    def test_an_empty_value_resolves_to_nothing(self):
        self.assertEqual(self.cfg.get('web', 'blank'), '[]')

    def test_a_later_duplicate_wins(self):
        self.assertEqual(self.cfg.get('web', 'dup'), 'second')

    def test_raw_and_missing_keys_are_as_before(self):
        self.assertEqual(self.cfg.raw('paths', 'logs'), '${data}/logs')
        with self.assertRaises(KeyError):
            self.cfg.get('web', 'nope')
        with self.assertRaises(KeyError):
            self.cfg.get('nosection', 'x')


class Errors(unittest.TestCase):
    def setUp(self):
        self.cfg = Config.load(TEXT)

    def test_cycles_are_reported_from_the_repeated_key(self):
        self.assertEqual(failure(self.cfg, 'loop', 'a'), 'cycle: loop.a -> loop.b -> loop.c -> loop.a')
        self.assertEqual(failure(self.cfg, 'loop', 'b'), 'cycle: loop.b -> loop.c -> loop.a -> loop.b')
        self.assertEqual(failure(self.cfg, 'loop', 'me'), 'cycle: loop.me -> loop.me')
        # Asked from outside the cycle: the loop itself, not the way in.
        self.assertEqual(failure(self.cfg, 'loop', 'into'), 'cycle: loop.a -> loop.b -> loop.c -> loop.a')

    def test_unterminated_and_unknown(self):
        self.assertEqual(failure(self.cfg, 'bad', 'open'), 'unterminated reference in bad.open')
        self.assertEqual(failure(self.cfg, 'bad', 'missing'), 'unknown key nowhere.key referenced from bad.missing')
        self.assertEqual(failure(self.cfg, 'bad', 'bare'), 'unknown key nothing referenced from bad.bare')

    def test_an_error_deeper_in_names_the_value_that_holds_it(self):
        cfg = Config.load('[a]\nx = ${b.y}\n[b]\ny = ${open\n')
        self.assertEqual(failure(cfg, 'a', 'x'), 'unterminated reference in b.y')
        cfg = Config.load('[a]\nx = ${b.y}\n[b]\ny = ${gone}\n')
        self.assertEqual(failure(cfg, 'a', 'x'), 'unknown key gone referenced from b.y')


if __name__ == '__main__':
    unittest.main()
