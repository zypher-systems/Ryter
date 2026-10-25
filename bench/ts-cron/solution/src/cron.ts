export class CronError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "CronError";
  }
}

export interface Schedule {
  minutes: number[];
  hours: number[];
  days: number[];
  months: number[];
  weekdays: number[];
  /** Whether the day field was anything but `*`. */
  dayRestricted: boolean;
  /** Whether the weekday field was anything but `*`. */
  weekdayRestricted: boolean;
}

interface Field {
  name: string;
  min: number;
  max: number;
  names?: string[];
}

const FIELDS: Field[] = [
  { name: "minute", min: 0, max: 59 },
  { name: "hour", min: 0, max: 23 },
  { name: "day", min: 1, max: 31 },
  { name: "month", min: 1, max: 12, names: ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"] },
  { name: "weekday", min: 0, max: 7, names: ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"] },
];

function fail(message: string): never {
  throw new CronError(message);
}

function value(text: string, f: Field): number {
  if (/^\d+$/.test(text)) {
    const v = Number(text);
    if (v < f.min || v > f.max) fail(`${f.name} out of range: ${text}`);
    return v;
  }
  const at = f.names?.indexOf(text.toUpperCase()) ?? -1;
  if (at < 0) fail(`unknown name in ${f.name}: ${text}`);
  return at + (f.name === "month" ? 1 : 0);
}

function field(text: string, f: Field): number[] {
  const out = new Set<number>();
  for (const item of text.split(",")) {
    const [rangeText, stepText] = item.split("/", 2);
    let step = 1;
    if (item.includes("/")) {
      if (!/^\d+$/.test(stepText) || Number(stepText) === 0) fail(`bad step in ${f.name}: ${item}`);
      step = Number(stepText);
    }
    let lo: number;
    let hi: number;
    if (rangeText === "*") {
      lo = f.min;
      hi = f.max;
    } else if (rangeText.includes("-")) {
      const [a, b] = rangeText.split("-", 2);
      lo = value(a, f);
      hi = value(b, f);
      if (lo > hi) fail(`bad range in ${f.name}: ${rangeText}`);
    } else {
      lo = value(rangeText, f);
      hi = item.includes("/") ? f.max : lo;
    }
    for (let v = lo; v <= hi; v += step) out.add(f.name === "weekday" && v === 7 ? 0 : v);
  }
  return [...out].sort((a, b) => a - b);
}

/** The schedule an expression describes. */
export function parse(expr: string): Schedule {
  const parts = expr.trim().split(/\s+/).filter((p) => p !== "");
  if (parts.length !== 5) fail(`expected 5 fields, got ${parts.length}`);
  const [minutes, hours, days, months, weekdays] = parts.map((p, i) => field(p, FIELDS[i]));
  return {
    minutes,
    hours,
    days,
    months,
    weekdays,
    dayRestricted: parts[2] !== "*",
    weekdayRestricted: parts[4] !== "*",
  };
}

function dayMatches(s: Schedule, t: Date): boolean {
  const day = s.days.includes(t.getUTCDate());
  const weekday = s.weekdays.includes(t.getUTCDay());
  if (s.dayRestricted && s.weekdayRestricted) return day || weekday;
  if (s.dayRestricted) return day;
  if (s.weekdayRestricted) return weekday;
  return true;
}

/** The first run strictly after `after`, in UTC. */
export function next(schedule: Schedule, after: Date): Date {
  const t = new Date(after.getTime());
  t.setUTCSeconds(0, 0);
  t.setUTCMinutes(t.getUTCMinutes() + 1);
  const limit = new Date(after.getTime());
  limit.setUTCFullYear(limit.getUTCFullYear() + 5);
  while (t.getTime() <= limit.getTime()) {
    if (!schedule.months.includes(t.getUTCMonth() + 1)) {
      t.setUTCMonth(t.getUTCMonth() + 1, 1);
      t.setUTCHours(0, 0, 0, 0);
      continue;
    }
    if (!dayMatches(schedule, t)) {
      t.setUTCDate(t.getUTCDate() + 1);
      t.setUTCHours(0, 0, 0, 0);
      continue;
    }
    if (!schedule.hours.includes(t.getUTCHours())) {
      t.setUTCHours(t.getUTCHours() + 1, 0, 0, 0);
      continue;
    }
    if (!schedule.minutes.includes(t.getUTCMinutes())) {
      t.setUTCMinutes(t.getUTCMinutes() + 1, 0, 0);
      continue;
    }
    return t;
  }
  fail("no match within 5 years");
}
