import assert from "node:assert/strict";
import { test } from "node:test";

import { debounce, throttle } from "../src/index.ts";
import { FakeClock } from "./clock.ts";

/** Each run of `fn`, as `[time, argument]`. */
function record(clock: FakeClock) {
  const runs: [number, number][] = [];
  const fn = (n: number) => {
    runs.push([clock.now(), n]);
    return n * 10;
  };
  return { runs, fn };
}

test("trailing: the wait restarts on every call", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const d = debounce(fn, 100, { clock });
  d(1);
  clock.tick(50);
  d(2);
  clock.tick(40);
  d(3);
  clock.tick(99);
  assert.deepEqual(runs, []);
  clock.tick(1);
  assert.deepEqual(runs, [[190, 3]]);
  assert.equal(d.pending(), false);
});

test("the debounced function returns the last result", () => {
  const clock = new FakeClock();
  const { fn } = record(clock);
  const d = debounce(fn, 100, { clock });
  assert.equal(d(1), undefined);
  clock.tick(100);
  assert.equal(d(2), 10);
  clock.tick(100);
  assert.equal(d(3), 20);
  const lead = debounce(fn, 100, { clock, leading: true });
  assert.equal(lead(4), 40);
  assert.equal(lead(5), 40);
});

test("leading only: nothing at the end of the burst, a new burst runs again", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const d = debounce(fn, 100, { clock, leading: true, trailing: false });
  d(1);
  clock.tick(50);
  d(2);
  clock.tick(90);
  d(3);
  assert.equal(d.pending(), false);
  clock.tick(300);
  d(4);
  assert.deepEqual(runs, [[0, 1], [440, 4]]);
});

test("leading and trailing: the trailing run needs a second call", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const d = debounce(fn, 100, { clock, leading: true });
  d(1);
  assert.equal(d.pending(), false);
  clock.tick(150);
  assert.deepEqual(runs, [[0, 1]]);
  d(2);
  clock.tick(30);
  d(3);
  assert.equal(d.pending(), true);
  clock.tick(100);
  assert.deepEqual(runs, [[0, 1], [150, 2], [280, 3]]);
});

test("maxWait: a long burst still runs every maxWait ms", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const d = debounce(fn, 100, { clock, maxWait: 250 });
  for (let t = 0; t <= 420; t += 60) {
    d(t);
    clock.tick(60);
  }
  clock.tick(200);
  assert.deepEqual(runs, [[250, 240], [500, 420]]);
});

test("maxWait with leading: the first call runs, then every maxWait, then the tail", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const d = debounce(fn, 100, { clock, leading: true, maxWait: 200 });
  for (let t = 0; t < 400; t += 50) {
    d(t);
    clock.tick(50);
  }
  // The call at 350 was used by the run at 400, so nothing trails it.
  assert.deepEqual(runs, [[0, 0], [200, 150], [400, 350]]);
  clock.tick(20);
  d(420);
  clock.tick(200);
  assert.deepEqual(runs, [[0, 0], [200, 150], [400, 350], [520, 420]]);
});

test("flush runs the waiting call now and returns its result", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const d = debounce(fn, 100, { clock });
  d(1);
  clock.tick(30);
  assert.equal(d.flush(), 10);
  assert.deepEqual(runs, [[30, 1]]);
  assert.equal(d.pending(), false);
  clock.tick(200);
  assert.deepEqual(runs, [[30, 1]], "the timer must not run it again");
  assert.equal(d.flush(), 10, "nothing waiting: the last result");
});

test("cancel forgets the burst, so the next call is a leading one", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const d = debounce(fn, 100, { clock, leading: true });
  d(1);
  d(2);
  d.cancel();
  assert.equal(d.pending(), false);
  clock.tick(10);
  d(3);
  clock.tick(200);
  assert.deepEqual(runs, [[0, 1], [10, 3]]);
});

test("throttle: leading, then at most once per wait, then the tail", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const t = throttle(fn, 100, { clock });
  t(0);
  clock.tick(30);
  t(30);
  clock.tick(30);
  t(60);
  clock.tick(30);
  t(90);
  clock.tick(60);
  t(150);
  clock.tick(300);
  assert.deepEqual(runs, [[0, 0], [100, 90], [250, 150]]);
});

test("throttle without leading waits for the first wait", () => {
  const clock = new FakeClock();
  const { runs, fn } = record(clock);
  const t = throttle(fn, 100, { clock, leading: false });
  t(1);
  clock.tick(50);
  t(2);
  clock.tick(50);
  assert.deepEqual(runs, [[100, 2]]);
  t(3);
  clock.tick(100);
  assert.deepEqual(runs, [[100, 2], [200, 3]]);
});

test("the global clock is the default", () => {
  const d = debounce(() => 1, 10);
  assert.equal(d(), undefined);
  assert.equal(d.pending(), true);
  d.cancel();
  assert.equal(d.pending(), false);
});
