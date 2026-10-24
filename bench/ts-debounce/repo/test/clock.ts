import type { Clock } from "../src/timing.ts";

interface Timer {
  id: number;
  due: number;
  fn: () => void;
}

/** A clock that moves only when told to, running timers as it passes them. */
export class FakeClock implements Clock {
  time = 0;
  timers: Timer[] = [];
  nextId = 1;

  now(): number {
    return this.time;
  }

  setTimeout(fn: () => void, ms: number): unknown {
    const timer = { id: this.nextId++, due: this.time + Math.max(0, ms), fn };
    this.timers.push(timer);
    return timer.id;
  }

  clearTimeout(id: unknown): void {
    this.timers = this.timers.filter((t) => t.id !== id);
  }

  /** Advance `ms`, running every timer that comes due, in due order. */
  tick(ms: number): void {
    const end = this.time + ms;
    for (;;) {
      const next = this.timers
        .filter((t) => t.due <= end)
        .sort((a, b) => a.due - b.due || a.id - b.id)[0];
      if (!next) break;
      this.timers = this.timers.filter((t) => t.id !== next.id);
      this.time = next.due;
      next.fn();
    }
    this.time = end;
  }
}
