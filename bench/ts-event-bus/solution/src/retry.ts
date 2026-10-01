import type { EventBus } from "./bus.ts";

export interface RetryOptions {
  /** How many times to call `fn` before giving up. */
  attempts: number;
  /** Told of each retry (`"retry"`) and of giving up (`"gave-up"`). */
  bus?: EventBus;
}

/** Call `fn` until it succeeds, at most `options.attempts` times. */
export async function retry<T>(fn: () => Promise<T>, options: RetryOptions): Promise<T> {
  const { attempts, bus } = options;
  if (attempts < 1) {
    throw new RangeError("attempts must be at least 1");
  }
  for (let attempt = 1; ; attempt++) {
    try {
      return await fn();
    } catch (error) {
      if (attempt >= attempts) {
        bus?.emit("gave-up", { attempts, error });
        throw error;
      }
      bus?.emit("retry", { attempt, error });
    }
  }
}
