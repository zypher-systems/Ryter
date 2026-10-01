import assert from "node:assert/strict";
import { test } from "node:test";

import { EventBus, retry } from "../src/index.ts";

test("on and emit", () => {
  const bus = new EventBus();
  const seen: unknown[] = [];
  bus.on("saved", (payload) => seen.push(payload));
  bus.emit("saved", 1);
  bus.emit("other", 2);
  assert.deepEqual(seen, [1]);
});

test("once fires once", () => {
  const bus = new EventBus();
  let calls = 0;
  bus.once("saved", () => calls++);
  bus.emit("saved");
  bus.emit("saved");
  assert.equal(calls, 1);
});

test("emit says how many handlers it called", () => {
  const bus = new EventBus();
  bus.on("saved", () => {});
  bus.on("saved", () => {});
  assert.equal(bus.emit("saved"), 2);
  assert.equal(bus.emit("nobody"), 0);
});

test("retry reports each retry on the bus", async () => {
  const bus = new EventBus();
  const attempts: number[] = [];
  bus.on("retry", (payload) => attempts.push((payload as { attempt: number }).attempt));
  let calls = 0;
  const value = await retry(
    async () => {
      calls++;
      if (calls < 3) throw new Error("not yet");
      return "done";
    },
    { attempts: 5, bus },
  );
  assert.equal(value, "done");
  assert.deepEqual(attempts, [1, 2]);
});
