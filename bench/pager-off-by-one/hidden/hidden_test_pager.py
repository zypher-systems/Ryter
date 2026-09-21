import unittest

from pager import paginate


class TestPaginateHidden(unittest.TestCase):
    def test_last_partial_page(self):
        self.assertEqual(paginate(list(range(10)), 4, 3), [9])

    def test_past_the_end(self):
        self.assertEqual(paginate(list(range(10)), 5, 3), [])

    def test_page_zero_is_an_error(self):
        with self.assertRaises(ValueError):
            paginate([1, 2], 0, 1)

    def test_size_zero_is_an_error(self):
        with self.assertRaises(ValueError):
            paginate([1, 2], 1, 0)


if __name__ == "__main__":
    unittest.main()
