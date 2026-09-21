import unittest

from textutil import slugify


class TestSlugifyHidden(unittest.TestCase):
    def test_runs_and_edges(self):
        self.assertEqual(slugify("  Hello,  World!  "), "hello-world")

    def test_accents(self):
        self.assertEqual(slugify("Café au lait"), "cafe-au-lait")

    def test_repeated_separators(self):
        self.assertEqual(slugify("a--b__c"), "a-b-c")

    def test_empty(self):
        self.assertEqual(slugify(""), "")
        self.assertEqual(slugify("!!!"), "")


if __name__ == "__main__":
    unittest.main()
