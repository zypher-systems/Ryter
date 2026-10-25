import unittest

from limiter import Decision, TokenBucket


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now


class Basics(unittest.TestCase):
    def test_a_key_starts_full_and_runs_dry(self):
        clock = Clock()
        bucket = TokenBucket(3, 1.0, clock)
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 2))
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 1))
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 0))
        self.assertFalse(bucket.allow('a').allowed)

    def test_keys_are_separate(self):
        bucket = TokenBucket(1, 1.0, Clock())
        self.assertTrue(bucket.allow('a').allowed)
        self.assertTrue(bucket.allow('b').allowed)
        self.assertFalse(bucket.allow('a').allowed)

    def test_tokens_come_back_with_time(self):
        clock = Clock()
        bucket = TokenBucket(2, 1.0, clock)
        bucket.allow('a', 2)
        clock.now = 1.5
        self.assertEqual(bucket.allow('a'), Decision(True, 0.0, 0))

    def test_a_refusal_says_when_to_retry(self):
        clock = Clock()
        bucket = TokenBucket(1, 2.0, clock)
        bucket.allow('a')
        self.assertEqual(bucket.allow('a'), Decision(False, 0.5, 0))


if __name__ == '__main__':
    unittest.main()
