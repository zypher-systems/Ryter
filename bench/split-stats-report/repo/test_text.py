import unittest

from stats.text import pad_left, pad_right


class TextTest(unittest.TestCase):
    def test_pad_right(self):
        self.assertEqual(pad_right("ab", 4), "ab  ")
        self.assertEqual(pad_right("abcdef", 4), "abcdef")

    def test_pad_left(self):
        self.assertEqual(pad_left("7", 3), "  7")
        self.assertEqual(pad_left("1234", 3), "1234")


if __name__ == "__main__":
    unittest.main()
