import assert from "node:assert/strict";
import { test } from "node:test";

import { debounce } from "../src/index.ts";
import { FakeClock } from "./clock.ts";

test("trailing: runs once the calls have stopped, with the last arguments", () => {
  const clock = new FakeClock();
  const calls: number[] = [];
  const d = debounce((n: number) => calls.push(n), 100, { clock });
  d(1);
  clock.tick(50);
  d(2);
  clock.tick(99);
  assert.deepEqual(calls, []);
  clock.tick(1);
  assert.deepEqual(calls, [2]);
});

test("leading: runs at once on the first call", () => {
  const clock = new FakeClock();
  const calls: number[] = [];
  const d = debounce((n: number) => calls.push(n), 100, { clock, leading: true, trailing: false });
  d(1);
  d(2);
  assert.deepEqual(calls, [1]);
  clock.tick(200);
  assert.deepEqual(calls, [1]);
});

test("cancel drops the waiting call", () => {
  const clock = new FakeClock();
  let calls = 0;
  const d = debounce(() => calls++, 100, { clock });
  d();
  assert.equal(d.pending(), true);
  d.cancel();
  assert.equal(d.pending(), false);
  clock.tick(200);
  assert.equal(calls, 0);
});
