"""A token bucket per key, refilled by the second."""
import time
from dataclasses import dataclass


@dataclass(frozen=True)
class Decision:
    allowed: bool
    retry_after: float
    remaining: int


class TokenBucket:
    def __init__(self, capacity, rate, clock=time.monotonic):
        self.capacity = capacity
        self.rate = rate
        self.clock = clock
        self._buckets = {}

    def allow(self, key, cost=1):
        now = self.clock()
        tokens, last = self._buckets.get(key, (self.capacity, now))
        whole_seconds = int(now - last)
        if whole_seconds:
            tokens = min(self.capacity, tokens + whole_seconds * self.rate)
            last += whole_seconds
        if tokens >= cost:
            tokens -= cost
            self._buckets[key] = (tokens, last)
            return Decision(True, 0.0, int(tokens))
        self._buckets[key] = (tokens, last)
        return Decision(False, 0.0, int(tokens))
