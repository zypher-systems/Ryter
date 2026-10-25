import assert from "node:assert/strict";
import { test } from "node:test";

import { applyPatch, resolve } from "../src/index.ts";
import type { Json } from "../src/index.ts";

const doc = (): Json => ({ name: "app", tags: ["a", "b"], "a/b": { "m~n": 1 } });

test("add, replace and remove", () => {
  const out = applyPatch(doc(), [
    { op: "add", path: "/port", value: 80 },
    { op: "replace", path: "/name", value: "web" },
    { op: "remove", path: "/tags/0" },
  ]);
  assert.deepEqual(out, { name: "web", port: 80, tags: ["b"], "a/b": { "m~n": 1 } });
});

test("resolve walks objects and arrays", () => {
  assert.equal(resolve(doc(), "/tags/1"), "b");
  assert.equal(resolve(doc(), "/name"), "app");
  assert.deepEqual(resolve(doc(), ""), doc());
});

test("escapes name slashes and tildes", () => {
  assert.equal(resolve(doc(), "/a~1b/m~0n"), 1);
});

test("a dash appends to an array", () => {
  const out = applyPatch(doc(), [{ op: "add", path: "/tags/-", value: "c" }]);
  assert.deepEqual(resolve(out, "/tags"), ["a", "b", "c"]);
});
