import unittest

from stats.parse import parse_rows


class ParseTest(unittest.TestCase):
    def test_rows(self):
        self.assertEqual(parse_rows(["a,1", "b,2.5"]), [("a", 1.0), ("b", 2.5)])

    def test_header_blank_and_comment_lines_are_skipped(self):
        lines = ["name,value", "", "# a comment", " a , 3 "]
        self.assertEqual(parse_rows(lines), [("a", 3.0)])

    def test_a_bad_line_is_named(self):
        with self.assertRaises(ValueError) as caught:
            parse_rows(["a,1", "b,two"])
        self.assertTrue(str(caught.exception).startswith("line 2:"))


if __name__ == "__main__":
    unittest.main()
