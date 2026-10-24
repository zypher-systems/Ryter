/** Where time comes from: the global timers, or a fake in tests. */
export interface Clock {
  now(): number;
  setTimeout(fn: () => void, ms: number): unknown;
  clearTimeout(id: unknown): void;
}

export const globalClock: Clock = {
  now: () => Date.now(),
  setTimeout: (fn, ms) => setTimeout(fn, ms),
  clearTimeout: (id) => clearTimeout(id as Parameters<typeof clearTimeout>[0]),
};

export interface DebounceOptions {
  leading?: boolean;
  trailing?: boolean;
  maxWait?: number;
  clock?: Clock;
}

export interface Debounced<A extends unknown[], R> {
  (...args: A): R | undefined;
  cancel(): void;
  flush(): R | undefined;
  pending(): boolean;
}

/** Runs `fn` once the calls have stopped for `wait` ms. */
export function debounce<A extends unknown[], R>(fn: (...args: A) => R, wait: number): (...args: A) => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  return (...args: A) => {
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = undefined;
      fn(...args);
    }, wait);
  };
}

export function throttle<A extends unknown[], R>(_fn: (...args: A) => R, _wait: number): Debounced<A, R> {
  throw new Error("not implemented");
}
