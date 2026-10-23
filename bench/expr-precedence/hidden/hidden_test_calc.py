import unittest

from calc import CalcError, evaluate


def failure(text):
    try:
        evaluate(text)
    except CalcError as e:
        return e.message, e.position
    raise AssertionError(f'{text!r} evaluated')


class Precedence(unittest.TestCase):
    def test_the_four_levels(self):
        self.assertEqual(evaluate('1+2*3-4/2'), 5.0)
        self.assertEqual(evaluate('2*3^2'), 18)
        self.assertEqual(evaluate('10-4-3'), 3)
        self.assertEqual(evaluate('2*3%4'), 2)
        self.assertEqual(evaluate('7%3*2'), 2)

    def test_unary_minus_sits_between_product_and_power(self):
        self.assertEqual(evaluate('-2^2'), -4)
        self.assertEqual(evaluate('(-2)^2'), 4)
        self.assertEqual(evaluate('2*-3'), -6)
        self.assertEqual(evaluate('--3'), 3)
        self.assertEqual(evaluate('-(2+3)'), -5)
        self.assertEqual(evaluate('4--1'), 5)

    def test_power_is_right_associative_and_takes_a_signed_exponent(self):
        self.assertEqual(evaluate('2^3^2'), 512)
        self.assertEqual(evaluate('2^-1'), 0.5)
        self.assertEqual(evaluate('2^-2^-1'), 2 ** -(2 ** -1))
        self.assertEqual(evaluate('-2^-2'), -0.25)

    def test_numbers_and_types(self):
        self.assertEqual(evaluate('.5+.5'), 1.0)
        self.assertEqual(evaluate('1.5*2'), 3.0)
        v = evaluate('6/3')
        self.assertEqual(v, 2.0)
        self.assertIsInstance(v, float)
        v = evaluate('2^10')
        self.assertEqual(v, 1024)
        self.assertIsInstance(v, int)
        self.assertEqual(evaluate(' ( 1 + 2 ) * 3 '), 9)
        self.assertEqual(evaluate('((((7))))'), 7)
        self.assertEqual(evaluate('-7%3'), 2)


class Errors(unittest.TestCase):
    def test_early_end(self):
        self.assertEqual(failure(''), ('unexpected end of input', 0))
        self.assertEqual(failure('   '), ('unexpected end of input', 3))
        self.assertEqual(failure('('), ('unexpected end of input', 1))
        self.assertEqual(failure('1*'), ('unexpected end of input', 2))
        self.assertEqual(failure('2^'), ('unexpected end of input', 2))
        self.assertEqual(failure('-'), ('unexpected end of input', 1))

    def test_unexpected_tokens_point_at_the_character(self):
        self.assertEqual(failure('1 2'), ("unexpected '2'", 2))
        self.assertEqual(failure('2(3)'), ("unexpected '('", 1))
        self.assertEqual(failure('1+*2'), ("unexpected '*'", 2))
        self.assertEqual(failure(')'), ("unexpected ')'", 0))
        self.assertEqual(failure('1$'), ("unexpected '$'", 1))
        self.assertEqual(failure('1 + a'), ("unexpected 'a'", 4))
        self.assertEqual(failure('*1'), ("unexpected '*'", 0))

    def test_an_open_group_points_where_the_close_should_be(self):
        self.assertEqual(failure('(1+2'), ("expected ')'", 4))
        self.assertEqual(failure('(1+2 3'), ("expected ')'", 5))
        self.assertEqual(failure('((1)'), ("expected ')'", 4))

    def test_division_by_zero_points_at_the_operator(self):
        self.assertEqual(failure('1/0'), ('division by zero', 1))
        self.assertEqual(failure('10 % (2-2)'), ('division by zero', 3))
        self.assertEqual(failure('1/(1-1)'), ('division by zero', 1))

    def test_the_first_error_wins(self):
        self.assertEqual(failure('1/0 $'), ('division by zero', 1))
        self.assertEqual(failure('$ 1/0'), ("unexpected '$'", 0))


if __name__ == '__main__':
    unittest.main()
