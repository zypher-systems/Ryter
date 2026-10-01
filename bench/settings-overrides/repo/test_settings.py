import unittest

import settings

DEFAULTS = {"debug": False, "db": {"host": "localhost", "port": 5432}}


class SettingsTest(unittest.TestCase):
    def test_no_overrides_gives_the_defaults(self):
        self.assertEqual(settings.load(DEFAULTS, {"HOME": "/root"}), DEFAULTS)

    def test_a_top_level_override(self):
        self.assertEqual(settings.load(DEFAULTS, {"APP_DEBUG": "true"})["debug"], True)

    def test_a_nested_override_keeps_its_siblings(self):
        got = settings.load(DEFAULTS, {"APP_DB__PORT": "5433"})
        self.assertEqual(got["db"], {"host": "localhost", "port": 5433})


if __name__ == "__main__":
    unittest.main()
