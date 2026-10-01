import dataclasses
import unittest

from stats.parse import parse_rows
from stats.report import render, report
from stats.summarize import summarize

HEADER = "name  count       total        mean"


class HiddenStatsTest(unittest.TestCase):
    def test_parse_ignores_whitespace_blanks_and_comments(self):
        lines = ["  # heading", "NAME,Value", "", "  alpha , 2 ", "\t", "beta,-1.5", "#x,1"]
        self.assertEqual(parse_rows(lines), [("alpha", 2.0), ("beta", -1.5)])

    def test_only_a_first_data_line_is_a_header(self):
        self.assertEqual(parse_rows(["name,value", "a,1"]), [("a", 1.0)])
        with self.assertRaises(ValueError) as caught:
            parse_rows(["a,1", "name,value"])
        self.assertTrue(str(caught.exception).startswith("line 2:"), caught.exception)

    def test_bad_lines_are_named_counting_every_line(self):
        for lines, n in [
            (["", "# c", "a;1"], 3),
            (["a,1", ",2"], 2),
            (["a,1,2"], 1),
            (["a,"], 1),
            (["a,1", "", "b,x"], 3),
        ]:
            with self.assertRaises(ValueError) as caught:
                parse_rows(lines)
            self.assertTrue(str(caught.exception).startswith(f"line {n}:"), caught.exception)

    def test_parse_takes_any_iterable(self):
        self.assertEqual(parse_rows(iter(["a,1"])), [("a", 1.0)])

    def test_summaries_keep_first_seen_order_and_are_frozen(self):
        got = summarize([("b", 2.0), ("a", 1.0), ("b", 4.0), ("a", -1.0)])
        self.assertEqual(list(got), ["b", "a"])
        b, a = got["b"], got["a"]
        self.assertEqual((b.count, b.total, b.mean, b.minimum, b.maximum), (2, 6.0, 3.0, 2.0, 4.0))
        self.assertEqual((a.count, a.total, a.mean, a.minimum, a.maximum), (2, 0.0, 0.0, -1.0, 1.0))
        with self.assertRaises(dataclasses.FrozenInstanceError):
            b.count = 5

    def test_render_of_nothing_is_the_header(self):
        self.assertEqual(render({}), [HEADER])
        self.assertEqual(report([]), [HEADER])

    def test_the_whole_report(self):
        # Totals and means that need no rounding, and a tie on the total.
        lines = [
            "name,value",
            "widgets,10",
            "bolts,2.5",
            "widgets,5",
            "# returns",
            "nuts,15",
            "bolts,0.5",
            "a-very-long-name,1",
        ]
        self.assertEqual(
            report(lines),
            [
                "name              count       total        mean",
                "nuts                  1       15.00       15.00",
                "widgets               2       15.00        7.50",
                "bolts                 2        3.00        1.50",
                "a-very-long-name      1        1.00        1.00",
            ],
        )

    def test_no_trailing_whitespace_and_wide_numbers_fit(self):
        got = report(["a,123456789.5", "b,1"])
        self.assertEqual(got[1], "a         1  123456789.50  123456789.50")
        self.assertTrue(all(line == line.rstrip() for line in got))


if __name__ == "__main__":
    unittest.main()
