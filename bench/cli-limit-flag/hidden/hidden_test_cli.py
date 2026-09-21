import io
import unittest

from cli import main


class TestCliHidden(unittest.TestCase):
    def run_cli(self, *argv):
        out = io.StringIO()
        main(list(argv), out)
        return out.getvalue()

    def test_no_limit_prints_everything(self):
        self.assertEqual(self.run_cli("data.txt"), "one\ntwo\nthree\n")

    def test_zero(self):
        self.assertEqual(self.run_cli("data.txt", "--limit", "0"), "")

    def test_more_than_there_are(self):
        self.assertEqual(self.run_cli("data.txt", "--limit", "9"), "one\ntwo\nthree\n")

    def test_negative_is_a_usage_error(self):
        with self.assertRaises(SystemExit) as e:
            self.run_cli("data.txt", "--limit", "-1")
        self.assertEqual(e.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
