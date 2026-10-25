import assert from "node:assert/strict";
import { test } from "node:test";

import { next, parse } from "../src/index.ts";

const at = (iso: string) => new Date(iso);

test("every weekday morning", () => {
  const s = parse("0 9 * * 1-5");
  assert.deepEqual(s.weekdays, [1, 2, 3, 4, 5]);
  // From a Saturday: Monday at nine.
  assert.equal(next(s, at("2026-10-24T12:00:00Z")).toISOString(), "2026-10-26T09:00:00.000Z");
});

test("steps and names", () => {
  assert.deepEqual(parse("*/15 * * * *").minutes, [0, 15, 30, 45]);
  assert.deepEqual(parse("0 0 1 jan,JUL *").months, [1, 7]);
});

test("strictly after, at the top of the minute", () => {
  const s = parse("30 8 * * *");
  assert.equal(next(s, at("2026-03-01T08:30:00Z")).toISOString(), "2026-03-02T08:30:00.000Z");
  assert.equal(next(s, at("2026-03-01T08:29:59Z")).toISOString(), "2026-03-01T08:30:00.000Z");
});
