import unittest

from stats.summarize import summarize


class SummarizeTest(unittest.TestCase):
    def test_one_name(self):
        s = summarize([("a", 1.0), ("a", 3.0)])["a"]
        self.assertEqual((s.count, s.total, s.mean, s.minimum, s.maximum), (2, 4.0, 2.0, 1.0, 3.0))

    def test_no_rows(self):
        self.assertEqual(summarize([]), {})


if __name__ == "__main__":
    unittest.main()
