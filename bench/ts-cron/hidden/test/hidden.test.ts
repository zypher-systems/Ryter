import assert from "node:assert/strict";
import { test } from "node:test";

import { CronError, next, parse } from "../src/index.ts";

const at = (iso: string) => new Date(iso);
const run = (expr: string, from: string) => next(parse(expr), at(from)).toISOString();

function failure(fn: () => unknown): string {
  try {
    fn();
  } catch (e) {
    assert.ok(e instanceof CronError, `not a CronError: ${e}`);
    return e.message;
  }
  assert.fail("no error");
}

test("fields are sorted and deduplicated, with the restriction flags", () => {
  const s = parse("5,1,5 */6 * * 7,MON");
  assert.deepEqual(s.minutes, [1, 5]);
  assert.deepEqual(s.hours, [0, 6, 12, 18]);
  assert.deepEqual(s.days, Array.from({ length: 31 }, (_, i) => i + 1));
  assert.deepEqual(s.weekdays, [0, 1]);
  assert.equal(s.dayRestricted, false);
  assert.equal(s.weekdayRestricted, true);
  assert.equal(parse("* * */2 * *").dayRestricted, true);
  assert.equal(parse("* * 1-31 * *").dayRestricted, true);
});

test("steps over ranges and from a value", () => {
  assert.deepEqual(parse("1-10/3 * * * *").minutes, [1, 4, 7, 10]);
  assert.deepEqual(parse("* 20/2 * * *").hours, [20, 22]);
  assert.deepEqual(parse("* * 10/10 * *").days, [10, 20, 30]);
  assert.deepEqual(parse("* * * mar-jun/2 *").months, [3, 5]);
  assert.deepEqual(parse("* * * * MON-FRI").weekdays, [1, 2, 3, 4, 5]);
  assert.deepEqual(parse("* * * * FRI-SAT,0").weekdays, [0, 5, 6]);
  assert.deepEqual(parse("* * * * 5-7").weekdays, [0, 5, 6]);
  assert.deepEqual(parse("  0   0 1 1 *  ").minutes, [0]);
});

test("errors name the field", () => {
  assert.equal(failure(() => parse("60 * * * *")), "minute out of range: 60");
  assert.equal(failure(() => parse("* 24 * * *")), "hour out of range: 24");
  assert.equal(failure(() => parse("* * 0 * *")), "day out of range: 0");
  assert.equal(failure(() => parse("* * 32 * *")), "day out of range: 32");
  assert.equal(failure(() => parse("* * * 13 *")), "month out of range: 13");
  assert.equal(failure(() => parse("* * * * 8")), "weekday out of range: 8");
  assert.equal(failure(() => parse("* * 10-5 * *")), "bad range in day: 10-5");
  assert.equal(failure(() => parse("* * * FRI-MON *")), "bad range in month: FRI-MON".replace("bad range in month: FRI-MON", "unknown name in month: FRI"));
  assert.equal(failure(() => parse("* */0 * * *")), "bad step in hour: */0");
  assert.equal(failure(() => parse("*/x * * * *")), "bad step in minute: */x");
  assert.equal(failure(() => parse("1-5/ * * * *")), "bad step in minute: 1-5/");
  assert.equal(failure(() => parse("* * * JUN3 *")), "unknown name in month: JUN3");
  assert.equal(failure(() => parse("* * * * weds")), "unknown name in weekday: weds");
  assert.equal(failure(() => parse("mon * * * *")), "unknown name in minute: mon");
  assert.equal(failure(() => parse("* * * *")), "expected 5 fields, got 4");
  assert.equal(failure(() => parse("* * * * * *")), "expected 5 fields, got 6");
});

test("day of month and day of week: either, when both are restricted", () => {
  // The 13th or a Friday: from Thu 2026-11-12, Fri the 13th; then Wed the 18th? No: Fri the 20th.
  assert.equal(run("0 0 13 * 5", "2026-11-12T00:00:00Z"), "2026-11-13T00:00:00.000Z");
  assert.equal(run("0 0 13 * 5", "2026-11-13T00:00:00Z"), "2026-11-20T00:00:00.000Z");
  // Only the weekday restricted: the 13th means nothing.
  assert.equal(run("0 0 * * 5", "2026-11-13T00:00:00Z"), "2026-11-20T00:00:00.000Z");
  // Only the day restricted: the weekday means nothing.
  assert.equal(run("0 0 13 * *", "2026-11-13T00:00:00Z"), "2026-12-13T00:00:00.000Z");
  // `*/n` in the day field restricts it, so the weekday is an alternative.
  assert.equal(run("0 0 */10 * 1", "2026-11-13T00:00:00Z"), "2026-11-16T00:00:00.000Z");
  assert.equal(run("0 0 */10 * 1", "2026-11-16T00:00:00Z"), "2026-11-21T00:00:00.000Z");
});

test("rolling over hours, days, months and years", () => {
  assert.equal(run("0 0 1 * *", "2026-12-31T23:59:00Z"), "2027-01-01T00:00:00.000Z");
  assert.equal(run("30 14 * * *", "2026-01-31T15:00:00Z"), "2026-02-01T14:30:00.000Z");
  assert.equal(run("0 0 29 2 *", "2025-03-01T00:00:00Z"), "2028-02-29T00:00:00.000Z");
  assert.equal(run("0 0 31 * *", "2026-02-01T00:00:00Z"), "2026-03-31T00:00:00.000Z");
  assert.equal(run("59 23 * * *", "2026-06-15T23:59:00Z"), "2026-06-16T23:59:00.000Z");
  assert.equal(run("* * * * *", "2026-06-15T10:20:30.400Z"), "2026-06-15T10:21:00.000Z");
  assert.equal(run("0 12 * * SUN", "2026-10-25T12:00:00Z"), "2026-11-01T12:00:00.000Z");
  assert.equal(run("0 12 * * 7", "2026-10-25T11:00:00Z"), "2026-10-25T12:00:00.000Z");
});

test("the input date is not changed and a hopeless schedule says so", () => {
  const from = at("2026-06-15T10:20:30Z");
  const s = parse("0 0 * * *");
  next(s, from);
  assert.equal(from.toISOString(), "2026-06-15T10:20:30.000Z");
  assert.equal(failure(() => next(parse("0 0 30 2 *"), from)), "no match within 5 years");
  assert.equal(failure(() => next(parse("0 0 31 4,6,9,11 *"), from)), "no match within 5 years");
});
