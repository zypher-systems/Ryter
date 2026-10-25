import assert from "node:assert/strict";
import { test } from "node:test";

import { PatchError, applyPatch, resolve } from "../src/index.ts";
import type { Json, Op } from "../src/index.ts";

const doc = (): Json => ({ a: { b: [1, 2, 3], c: "x" }, list: [{ id: 1 }, { id: 2 }], "~": "tilde", "/": "slash" });

function failure(ops: Op[], input: Json = doc()): [string, number] {
  try {
    applyPatch(input, ops);
  } catch (e) {
    assert.ok(e instanceof PatchError, `not a PatchError: ${e}`);
    return [e.message, e.index];
  }
  assert.fail("applied");
}

test("the input is never changed, even when an operation fails", () => {
  const input = doc();
  const before = JSON.stringify(input);
  const out = applyPatch(input, [{ op: "add", path: "/a/d", value: 1 }, { op: "remove", path: "/list/0" }]);
  assert.equal(JSON.stringify(input), before);
  assert.notDeepEqual(out, input);
  failure([{ op: "add", path: "/a/d", value: 1 }, { op: "remove", path: "/nope" }], input);
  assert.equal(JSON.stringify(input), before);
});

test("pointer escapes, in order", () => {
  assert.equal(resolve(doc(), "/~0"), "tilde");
  assert.equal(resolve(doc(), "/~1"), "slash");
  const out = applyPatch(doc(), [{ op: "add", path: "/~01", value: "t1" }]);
  assert.equal((out as { [k: string]: Json })["~1"], "t1");
  assert.throws(() => resolve(doc(), "a/b"), (e: PatchError) => e.message === "invalid pointer: a/b" && e.index === -1);
  assert.throws(() => resolve(doc(), "/a/zz"), (e: PatchError) => e.message === "path not found: /a/zz" && e.index === -1);
});

test("add inserts into arrays, appends at the dash, and replaces the root", () => {
  let out = applyPatch(doc(), [{ op: "add", path: "/a/b/1", value: 9 }]);
  assert.deepEqual(resolve(out, "/a/b"), [1, 9, 2, 3]);
  out = applyPatch(doc(), [{ op: "add", path: "/a/b/3", value: 9 }]);
  assert.deepEqual(resolve(out, "/a/b"), [1, 2, 3, 9]);
  out = applyPatch(doc(), [{ op: "add", path: "", value: { fresh: true } }]);
  assert.deepEqual(out, { fresh: true });
  out = applyPatch(doc(), [{ op: "replace", path: "", value: 7 }]);
  assert.equal(out, 7);
  // An added value is not shared with the caller's object.
  const value: Json = { shared: [1] };
  out = applyPatch(doc(), [{ op: "add", path: "/v", value }]);
  (resolve(out, "/v/shared") as Json[]).push(2);
  assert.deepEqual(value, { shared: [1] });
});

test("remove and replace need the path to exist", () => {
  assert.deepEqual(failure([{ op: "remove", path: "/a/zz" }]), ["path not found: /a/zz", 0]);
  assert.deepEqual(failure([{ op: "remove", path: "/a/b/3" }]), ["path not found: /a/b/3", 0]);
  assert.deepEqual(failure([{ op: "add", path: "/a/c", value: 1 }, { op: "replace", path: "/a/zz", value: 1 }]), ["path not found: /a/zz", 1]);
  assert.deepEqual(failure([{ op: "remove", path: "" }]), ["path not found: ", 0]);
  const out = applyPatch(doc(), [{ op: "remove", path: "/a/b/0" }, { op: "replace", path: "/a/b/0", value: 20 }]);
  assert.deepEqual(resolve(out, "/a/b"), [20, 3]);
});

test("array indices are checked", () => {
  assert.deepEqual(failure([{ op: "add", path: "/a/b/4", value: 1 }]), ["index out of range: /a/b/4", 0]);
  assert.deepEqual(failure([{ op: "add", path: "/a/b/x", value: 1 }]), ["index out of range: /a/b/x", 0]);
  assert.deepEqual(failure([{ op: "add", path: "/a/b/01", value: 1 }]), ["index out of range: /a/b/01", 0]);
  assert.deepEqual(failure([{ op: "remove", path: "/a/b/-" }]), ["index out of range: /a/b/-", 0]);
  assert.deepEqual(failure([{ op: "replace", path: "/a/b/-", value: 1 }]), ["index out of range: /a/b/-", 0]);
  assert.deepEqual(failure([{ op: "test", path: "/a/b/-", value: 1 }]), ["index out of range: /a/b/-", 0]);
});

test("move and copy", () => {
  let out = applyPatch(doc(), [{ op: "move", from: "/a/c", path: "/moved" }]);
  assert.equal(resolve(out, "/moved"), "x");
  assert.throws(() => resolve(out, "/a/c"));
  out = applyPatch(doc(), [{ op: "move", from: "/a/b/0", path: "/a/b/-" }]);
  assert.deepEqual(resolve(out, "/a/b"), [2, 3, 1]);
  out = applyPatch(doc(), [{ op: "copy", from: "/list/0", path: "/list/-" }]);
  assert.deepEqual(resolve(out, "/list"), [{ id: 1 }, { id: 2 }, { id: 1 }]);
  (resolve(out, "/list/2") as { [k: string]: Json }).id = 99;
  assert.deepEqual(resolve(out, "/list/0"), { id: 1 });
  // Moving onto itself changes nothing; into its own child is refused.
  out = applyPatch(doc(), [{ op: "move", from: "/a", path: "/a" }]);
  assert.deepEqual(out, doc());
  assert.deepEqual(failure([{ op: "move", from: "/a", path: "/a/b/0" }]), ["cannot move into own child: /a -> /a/b/0", 0]);
  assert.deepEqual(failure([{ op: "move", from: "", path: "/x" }]), ["cannot move into own child:  -> /x", 0]);
  assert.deepEqual(failure([{ op: "copy", from: "/zz", path: "/x" }]), ["path not found: /zz", 0]);
  assert.deepEqual(failure([{ op: "move", path: "/x" }]), ["missing from in move", 0]);
  assert.deepEqual(failure([{ op: "copy", path: "/x" }]), ["missing from in copy", 0]);
});

test("test compares deeply, whatever the key order", () => {
  const out = applyPatch(doc(), [
    { op: "test", path: "/list", value: [{ id: 1 }, { id: 2 }] },
    { op: "test", path: "/a", value: { c: "x", b: [1, 2, 3] } },
    { op: "test", path: "/a/b/0", value: 1.0 },
    { op: "add", path: "/ok", value: true },
  ]);
  assert.equal(resolve(out, "/ok"), true);
  assert.deepEqual(failure([{ op: "test", path: "/a/b", value: [3, 2, 1] }]), ["test failed at /a/b", 0]);
  assert.deepEqual(failure([{ op: "test", path: "/a", value: { c: "x" } }]), ["test failed at /a", 0]);
  assert.deepEqual(failure([{ op: "test", path: "/a/c", value: null }]), ["test failed at /a/c", 0]);
  assert.deepEqual(failure([{ op: "add", path: "/n", value: 1 }, { op: "test", path: "/n", value: "1" }]), ["test failed at /n", 1]);
});

test("unknown operations and missing fields name the operation", () => {
  assert.deepEqual(failure([{ op: "add", path: "/a/c", value: 1 }, { op: "frob", path: "/a" }]), ["unknown op: frob", 1]);
  assert.deepEqual(failure([{ op: "add", path: "/x" }]), ["missing value in add", 0]);
  assert.deepEqual(failure([{ op: "replace", path: "/a" }]), ["missing value in replace", 0]);
  assert.deepEqual(failure([{ op: "test", path: "/a" }]), ["missing value in test", 0]);
  assert.deepEqual(failure([{ op: "add", path: "x", value: 1 }]), ["invalid pointer: x", 0]);
});
