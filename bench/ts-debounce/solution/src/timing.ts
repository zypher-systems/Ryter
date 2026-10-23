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

/** Runs `fn` once the calls have stopped for `wait` ms, or on the first
 * call of a burst (`leading`), or at least every `maxWait` ms of a burst. */
export function debounce<A extends unknown[], R>(
  fn: (...args: A) => R,
  wait: number,
  options: DebounceOptions = {},
): Debounced<A, R> {
  const { leading = false, trailing = true, maxWait, clock = globalClock } = options;
  let timer: unknown = undefined;
  let lastArgs: A | undefined;
  let lastCallTime: number | undefined;
  let lastInvokeTime = 0;
  let result: R | undefined;

  const invoke = (time: number): R | undefined => {
    const args = lastArgs as A;
    lastArgs = undefined;
    lastInvokeTime = time;
    result = fn(...args);
    return result;
  };

  const shouldInvoke = (time: number): boolean => {
    if (lastCallTime === undefined) return true;
    const sinceCall = time - lastCallTime;
    const sinceInvoke = time - lastInvokeTime;
    return sinceCall >= wait || sinceCall < 0 || (maxWait !== undefined && sinceInvoke >= maxWait);
  };

  const remaining = (time: number): number => {
    const sinceCall = time - (lastCallTime as number);
    const sinceInvoke = time - lastInvokeTime;
    const left = wait - sinceCall;
    return maxWait === undefined ? left : Math.min(left, maxWait - sinceInvoke);
  };

  const trailingEdge = (time: number): R | undefined => {
    if (timer !== undefined) clock.clearTimeout(timer);
    timer = undefined;
    if (trailing && lastArgs !== undefined) return invoke(time);
    lastArgs = undefined;
    return result;
  };

  const timerExpired = (): void => {
    const time = clock.now();
    if (shouldInvoke(time)) {
      timer = undefined;
      trailingEdge(time);
      return;
    }
    timer = clock.setTimeout(timerExpired, remaining(time));
  };

  const leadingEdge = (time: number): void => {
    lastInvokeTime = time;
    timer = clock.setTimeout(timerExpired, wait);
    if (leading) invoke(time);
  };

  const debounced = ((...args: A): R | undefined => {
    const time = clock.now();
    const invoking = shouldInvoke(time);
    lastArgs = args;
    lastCallTime = time;
    if (invoking) {
      if (timer === undefined) {
        leadingEdge(time);
        return result;
      }
      if (maxWait !== undefined) {
        clock.clearTimeout(timer);
        timer = clock.setTimeout(timerExpired, wait);
        invoke(time);
        return result;
      }
    }
    if (timer === undefined) timer = clock.setTimeout(timerExpired, wait);
    return result;
  }) as Debounced<A, R>;

  debounced.cancel = () => {
    if (timer !== undefined) clock.clearTimeout(timer);
    timer = undefined;
    lastArgs = undefined;
    lastCallTime = undefined;
    lastInvokeTime = 0;
  };
  debounced.flush = () => (timer === undefined ? result : trailingEdge(clock.now()));
  // A call waits only if something will run it: the trailing edge, or maxWait.
  debounced.pending = () => timer !== undefined && lastArgs !== undefined && (trailing || maxWait !== undefined);
  return debounced;
}

/** `debounce` that runs at most once per `wait`: leading and trailing by
 * default, `maxWait` fixed at `wait`. */
export function throttle<A extends unknown[], R>(
  fn: (...args: A) => R,
  wait: number,
  options: Omit<DebounceOptions, "maxWait"> = {},
): Debounced<A, R> {
  const { leading = true, trailing = true, clock } = options;
  return debounce(fn, wait, { leading, trailing, maxWait: wait, clock });
}
