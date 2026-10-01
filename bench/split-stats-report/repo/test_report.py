import unittest

from stats.report import report


class ReportTest(unittest.TestCase):
    def test_report(self):
        self.assertEqual(
            report(["a,1", "bb,2", "a,2"]),
            [
                "name  count       total        mean",
                "a         2        3.00        1.50",
                "bb        1        2.00        2.00",
            ],
        )


if __name__ == "__main__":
    unittest.main()
