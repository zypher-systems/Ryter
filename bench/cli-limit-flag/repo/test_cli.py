import io
import unittest

from cli import main


class TestCli(unittest.TestCase):
    def test_limit(self):
        out = io.StringIO()
        main(["data.txt", "--limit", "2"], out)
        self.assertEqual(out.getvalue(), "one\ntwo\n")


if __name__ == "__main__":
    unittest.main()
