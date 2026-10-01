import assert from "node:assert/strict";
import { test } from "node:test";

import { EventBus, retry } from "../src/index.ts";

test("on returns a function that removes the handler", () => {
  const bus = new EventBus();
  let calls = 0;
  const stop = bus.on("tick", () => calls++);
  bus.emit("tick");
  stop();
  bus.emit("tick");
  assert.equal(calls, 1);
});

test("off still removes a handler, and a once handler", () => {
  const bus = new EventBus();
  let calls = 0;
  const handler = () => calls++;
  bus.on("tick", handler);
  bus.off("tick", handler);
  bus.once("tick", handler);
  bus.off("tick", handler);
  assert.equal(bus.emit("tick"), 0);
  assert.equal(calls, 0);
});

test("handlers get the payload and the event's name", () => {
  const bus = new EventBus();
  const seen: unknown[] = [];
  bus.on("saved", (payload, event) => seen.push([payload, event]));
  bus.emit("saved", { id: 7 });
  assert.deepEqual(seen, [[{ id: 7 }, "saved"]]);
});

test("wildcard handlers get every event, after its own handlers", () => {
  const bus = new EventBus();
  const order: string[] = [];
  bus.on("*", (_payload, event) => order.push(`any:${event}`));
  bus.on("saved", () => order.push("saved"));
  assert.equal(bus.emit("saved"), 2);
  assert.equal(bus.emit("closed"), 1);
  assert.deepEqual(order, ["saved", "any:saved", "any:closed"]);
});

test("emitting the wildcard itself calls its handlers once", () => {
  const bus = new EventBus();
  let calls = 0;
  bus.on("*", () => calls++);
  assert.equal(bus.emit("*"), 1);
  assert.equal(calls, 1);
});

test("a throwing handler doesn't stop the others", () => {
  const bus = new EventBus();
  const ran: string[] = [];
  bus.on("saved", () => {
    ran.push("first");
    throw new Error("one");
  });
  bus.on("saved", () => ran.push("second"));
  bus.on("*", () => {
    ran.push("any");
    throw new Error("two");
  });
  assert.throws(
    () => bus.emit("saved"),
    (thrown: unknown) => {
      assert.ok(thrown instanceof AggregateError);
      assert.deepEqual(
        thrown.errors.map((e: Error) => e.message),
        ["one", "two"],
      );
      return true;
    },
  );
  assert.deepEqual(ran, ["first", "second", "any"]);
});

test("a once handler that throws is still removed", () => {
  const bus = new EventBus();
  bus.once("saved", () => {
    throw new Error("boom");
  });
  assert.throws(() => bus.emit("saved"), AggregateError);
  assert.equal(bus.emit("saved"), 0);
});

test("removing a handler during an emit skips nobody", () => {
  const bus = new EventBus();
  const ran: string[] = [];
  const stopFirst = bus.on("saved", () => {
    ran.push("first");
    stopFirst();
  });
  bus.on("saved", () => ran.push("second"));
  bus.once("saved", () => ran.push("third"));
  bus.on("saved", () => ran.push("fourth"));
  assert.equal(bus.emit("saved"), 4);
  assert.deepEqual(ran, ["first", "second", "third", "fourth"]);
  assert.equal(bus.emit("saved"), 2);
});

test("retry needs no bus", async () => {
  let calls = 0;
  const value = await retry(
    async () => {
      calls++;
      if (calls < 2) throw new Error("not yet");
      return 42;
    },
    { attempts: 2 },
  );
  assert.equal(value, 42);
});

test("retry reports the failed attempt and its error, then gives up", async () => {
  const bus = new EventBus();
  const events: unknown[] = [];
  bus.on("*", (payload, event) => events.push([event, payload]));
  const errors = [new Error("a"), new Error("b"), new Error("c")];
  let calls = 0;
  await assert.rejects(
    retry(
      async () => {
        throw errors[calls++];
      },
      { attempts: 3, bus },
    ),
    errors[2],
  );
  assert.equal(calls, 3);
  assert.deepEqual(events, [
    ["retry", { attempt: 1, error: errors[0] }],
    ["retry", { attempt: 2, error: errors[1] }],
    ["gave-up", { attempts: 3, error: errors[2] }],
  ]);
});

test("a first-time success emits nothing", async () => {
  const bus = new EventBus();
  let events = 0;
  bus.on("*", () => events++);
  assert.equal(await retry(async () => "ok", { attempts: 3, bus }), "ok");
  assert.equal(events, 0);
});

test("fewer than one attempt is a RangeError, and fn isn't called", async () => {
  let calls = 0;
  const fn = async () => {
    calls++;
    return 1;
  };
  await assert.rejects(retry(fn, { attempts: 0 }), RangeError);
  await assert.rejects(retry(fn, { attempts: -2 }), RangeError);
  assert.equal(calls, 0);
});
