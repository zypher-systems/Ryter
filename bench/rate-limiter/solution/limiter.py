"""A token bucket per key, refilled continuously, idle keys forgotten."""
import math
import time
from collections import OrderedDict
from dataclasses import dataclass


@dataclass(frozen=True)
class Decision:
    allowed: bool
    retry_after: float
    remaining: int


class TokenBucket:
    def __init__(self, capacity, rate, clock=time.monotonic, max_keys=1000):
        self.capacity = capacity
        self.rate = rate
        self.clock = clock
        self.max_keys = max_keys
        self._buckets = OrderedDict()

    def __len__(self):
        return len(self._buckets)

    def _refilled(self, key, now):
        if key not in self._buckets:
            return float(self.capacity)
        tokens, last = self._buckets[key]
        return min(float(self.capacity), tokens + (now - last) * self.rate)

    def allow(self, key, cost=1):
        if cost > self.capacity:
            raise ValueError(f'cost {cost} exceeds capacity {self.capacity}')
        now = self.clock()
        tokens = self._refilled(key, now)
        if key in self._buckets:
            self._buckets.move_to_end(key)
        elif len(self._buckets) >= self.max_keys and self.max_keys > 0:
            self._buckets.popitem(last=False)
        if tokens >= cost:
            tokens -= cost
            self._buckets[key] = (tokens, now)
            return Decision(True, 0.0, math.floor(tokens))
        self._buckets[key] = (tokens, now)
        if self.rate == 0:
            return Decision(False, math.inf, math.floor(tokens))
        wait = math.ceil((cost - tokens) / self.rate * 1000) / 1000
        return Decision(False, wait, math.floor(tokens))

    def peek(self, key):
        return math.floor(self._refilled(key, self.clock()))

    def reset(self, key):
        self._buckets.pop(key, None)
