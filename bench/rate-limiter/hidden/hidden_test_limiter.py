import math
import unittest

from limiter import Decision, TokenBucket


class Clock:
    def __init__(self, now=0.0):
        self.now = now

    def __call__(self):
        return self.now


class Refill(unittest.TestCase):
    def test_fractions_accumulate_and_exactly_enough_is_enough(self):
        clock = Clock()
        bucket = TokenBucket(2, 1.0, clock)
        bucket.allow('a', 2)
        clock.now = 0.5
        self.assertEqual(bucket.allow('a'), Decision(False, 0.5, 0))
        clock.now = 1.0
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 0))

    def test_remaining_drops_the_fraction_and_a_refusal_takes_nothing(self):
        clock = Clock()
        bucket = TokenBucket(5, 2.0, clock)
        bucket.allow('a', 5)
        clock.now = 1.25
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 1))
        self.assertEqual(bucket.allow('a', 2), Decision(False, 0.25, 1))
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 0))

    def test_tokens_never_exceed_capacity(self):
        clock = Clock()
        bucket = TokenBucket(2, 10.0, clock)
        bucket.allow('a')
        clock.now = 100.0
        self.assertEqual(bucket.allow('a', 2), Decision(True, 0.0, 0))
        self.assertFalse(bucket.allow('a').allowed)

    def test_retry_after_rounds_up_to_the_millisecond(self):
        clock = Clock()
        bucket = TokenBucket(1, 3.0, clock)
        bucket.allow('a')
        self.assertEqual(bucket.allow('a'), Decision(False, 0.334, 0))
        bucket = TokenBucket(4, 0.5, clock)
        bucket.allow('b', 4)
        self.assertEqual(bucket.allow('b', 3), Decision(False, 6.0, 0))

    def test_no_refill_means_retry_never(self):
        bucket = TokenBucket(1, 0, Clock())
        bucket.allow('a')
        d = bucket.allow('a')
        self.assertEqual((d.allowed, d.remaining), (False, 0))
        self.assertTrue(math.isinf(d.retry_after))

    def test_the_default_clock_works(self):
        bucket = TokenBucket(2, 1000.0)
        self.assertTrue(bucket.allow('a').allowed)


class Edges(unittest.TestCase):
    def test_a_cost_above_capacity_is_an_error_and_touches_nothing(self):
        bucket = TokenBucket(3, 1.0, Clock())
        with self.assertRaises(ValueError) as caught:
            bucket.allow('a', 5)
        self.assertEqual(str(caught.exception), 'cost 5 exceeds capacity 3')
        self.assertEqual(len(bucket), 0)
        self.assertEqual(bucket.peek('a'), 3)

    def test_peek_takes_nothing_and_makes_no_bucket(self):
        clock = Clock()
        bucket = TokenBucket(3, 1.0, clock)
        self.assertEqual(bucket.peek('a'), 3)
        self.assertEqual(len(bucket), 0)
        bucket.allow('a', 3)
        self.assertEqual(bucket.peek('a'), 0)
        clock.now = 1.5
        self.assertEqual(bucket.peek('a'), 1)
        self.assertEqual(bucket.peek('a'), 1)
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 0))

    def test_reset_makes_a_key_full_again(self):
        bucket = TokenBucket(1, 1.0, Clock())
        bucket.allow('a')
        self.assertFalse(bucket.allow('a').allowed)
        bucket.reset('a')
        self.assertEqual(len(bucket), 0)
        self.assertTrue(bucket.allow('a').allowed)
        bucket.reset('never-seen')


class Eviction(unittest.TestCase):
    def test_the_least_recently_used_key_is_forgotten(self):
        clock = Clock()
        bucket = TokenBucket(1, 0, clock, max_keys=2)
        bucket.allow('a')
        bucket.allow('b')
        self.assertEqual(len(bucket), 2)
        self.assertFalse(bucket.allow('a').allowed)   # a is now the most recent
        bucket.allow('c')                              # b goes
        self.assertEqual(len(bucket), 2)
        self.assertTrue(bucket.allow('b').allowed)     # b is back, full; a goes
        self.assertEqual(len(bucket), 2)
        self.assertTrue(bucket.allow('a').allowed)

    def test_peek_and_a_refused_cost_do_not_count_as_use(self):
        bucket = TokenBucket(2, 0, Clock(), max_keys=2)
        bucket.allow('a')
        bucket.allow('b')
        bucket.peek('a')
        with self.assertRaises(ValueError):
            bucket.allow('a', 3)
        bucket.allow('c')
        self.assertEqual(len(bucket), 2)
        self.assertEqual(bucket.peek('a'), 2)   # forgotten: full again
        self.assertEqual(bucket.peek('b'), 1)


if __name__ == '__main__':
    unittest.main()
