import copy
import unittest

import settings
from settings.env import overrides, parse
from settings.merge import deep_merge

DEFAULTS = {
    "debug": True,
    "name": "app",
    "db": {"host": "localhost", "port": 5432, "pool": {"size": 5, "timeout": 2.5}},
}


class HiddenSettingsTest(unittest.TestCase):
    def test_false_is_false(self):
        self.assertIs(parse("false"), False)
        self.assertIs(parse("FALSE"), False)
        self.assertIs(parse("True"), True)
        self.assertIs(settings.load(DEFAULTS, {"APP_DEBUG": "false"})["debug"], False)

    def test_numbers(self):
        self.assertEqual(parse("42"), 42)
        self.assertEqual(parse("-7"), -7)
        self.assertIsInstance(parse("-7"), int)
        self.assertEqual(parse("2.5"), 2.5)
        self.assertIsInstance(parse("2.5"), float)

    def test_other_text_stays_text(self):
        for text in ["localhost", "", "1.2.3", "truely", "0x10"]:
            self.assertEqual(parse(text), text)

    def test_only_the_prefix_counts_and_keys_are_lowered(self):
        env = {"APP_NAME": "shop", "HOME": "/root", "APPLE": "1", "app_name": "x"}
        self.assertEqual(overrides(env), {"name": "shop"})

    def test_double_underscore_nests(self):
        env = {"APP_DB__POOL__SIZE": "20", "APP_DB__HOST": "db.internal"}
        self.assertEqual(overrides(env), {"db": {"pool": {"size": 20}, "host": "db.internal"}})

    def test_deep_overrides_keep_every_sibling(self):
        got = settings.load(DEFAULTS, {"APP_DB__POOL__SIZE": "20"})
        self.assertEqual(
            got["db"], {"host": "localhost", "port": 5432, "pool": {"size": 20, "timeout": 2.5}}
        )
        self.assertEqual(got["name"], "app")

    def test_defaults_are_not_modified_at_any_depth(self):
        before = copy.deepcopy(DEFAULTS)
        got = settings.load(DEFAULTS, {"APP_DB__POOL__SIZE": "20", "APP_NAME": "shop"})
        self.assertEqual(DEFAULTS, before)
        got["db"]["pool"]["timeout"] = 99
        got["db"]["host"] = "changed"
        self.assertEqual(DEFAULTS, before, "the result shares a dictionary with the defaults")

    def test_merge_replaces_a_value_with_a_dictionary_and_back(self):
        self.assertEqual(deep_merge({"a": 1}, {"a": {"b": 2}}), {"a": {"b": 2}})
        self.assertEqual(deep_merge({"a": {"b": 2}}, {"a": 1}), {"a": 1})

    def test_a_new_key_is_added(self):
        self.assertEqual(settings.load({"a": 1}, {"APP_B__C": "x"}), {"a": 1, "b": {"c": "x"}})


if __name__ == "__main__":
    unittest.main()
