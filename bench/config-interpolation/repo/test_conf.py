import unittest

from conf import Config

TEXT = """
[db]
host = db.internal
port = 5432
url = postgres://${host}:${port}/app

[app]
database = ${db.url}
price = 5$$
"""


class References(unittest.TestCase):
    def setUp(self):
        self.cfg = Config.load(TEXT)

    def test_a_qualified_reference_reads_another_section(self):
        self.assertEqual(self.cfg.get('app', 'database'), 'postgres://db.internal:5432/app')

    def test_a_bare_reference_reads_the_same_section(self):
        self.assertEqual(self.cfg.get('db', 'url'), 'postgres://db.internal:5432/app')

    def test_a_doubled_dollar_is_one_dollar(self):
        self.assertEqual(self.cfg.get('app', 'price'), '5$')

    def test_raw_is_as_written(self):
        self.assertEqual(self.cfg.raw('db', 'url'), 'postgres://${host}:${port}/app')


if __name__ == '__main__':
    unittest.main()
