export interface RetryOptions {
  /** How many times to call `fn` before giving up. */
  attempts: number;
}

/** Call `fn` until it succeeds, at most `options.attempts` times. */
export async function retry<T>(fn: () => Promise<T>, options: RetryOptions): Promise<T> {
  let last: unknown;
  for (let attempt = 1; attempt <= options.attempts; attempt++) {
    try {
      return await fn();
    } catch (error) {
      last = error;
    }
  }
  throw last;
}
