import unittest

import textutil


class TestTextutil(unittest.TestCase):
    def test_title_case(self):
        self.assertEqual(textutil.title_case("hello world"), "Hello World")

    def test_slugify_basic(self):
        self.assertEqual(textutil.slugify("Hello World"), "hello-world")


if __name__ == "__main__":
    unittest.main()
