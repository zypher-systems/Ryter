import unittest

from report import summary


class TestReport(unittest.TestCase):
    def test_summary(self):
        self.assertEqual(summary([(2, 3), (1, 4)]), "total: 10")


if __name__ == "__main__":
    unittest.main()
