import unittest

from calc import CalcError, evaluate


class Precedence(unittest.TestCase):
    def test_multiplication_binds_tighter(self):
        self.assertEqual(evaluate('1+2*3'), 7)

    def test_parentheses_group(self):
        self.assertEqual(evaluate('(1+2)*3'), 9)

    def test_power(self):
        self.assertEqual(evaluate('2^3'), 8)

    def test_division_is_float(self):
        self.assertEqual(evaluate('7/2'), 3.5)


class Errors(unittest.TestCase):
    def test_an_early_end_says_where(self):
        with self.assertRaises(CalcError) as caught:
            evaluate('1+')
        self.assertEqual((caught.exception.message, caught.exception.position), ('unexpected end of input', 2))

    def test_a_stray_character_says_where(self):
        with self.assertRaises(CalcError) as caught:
            evaluate('1$')
        self.assertEqual((caught.exception.message, caught.exception.position), ("unexpected '$'", 1))


if __name__ == '__main__':
    unittest.main()
