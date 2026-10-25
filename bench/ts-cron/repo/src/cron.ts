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

const RANGES: [string, number, number][] = [
  ["minute", 0, 59],
  ["hour", 0, 23],
  ["day", 1, 31],
  ["month", 1, 12],
  ["weekday", 0, 7],
];

function field(text: string, min: number, max: number): number[] {
  const out = new Set<number>();
  for (const item of text.split(",")) {
    if (item === "*") {
      for (let v = min; v <= max; v++) out.add(v);
    } else if (item.includes("-")) {
      const [a, b] = item.split("-").map(Number);
      for (let v = a; v <= b; v++) out.add(v);
    } else {
      out.add(Number(item));
    }
  }
  return [...out].sort((a, b) => a - b);
}

/** The schedule an expression describes. */
export function parse(expr: string): Schedule {
  const parts = expr.trim().split(/\s+/);
  if (parts.length !== 5) throw new CronError(`expected 5 fields, got ${parts.length}`);
  const [minutes, hours, days, months, weekdays] = parts.map((p, i) => field(p, RANGES[i][1], RANGES[i][2]));
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

/** The first run strictly after `after`, in UTC. */
export function next(schedule: Schedule, after: Date): Date {
  const t = new Date(after.getTime());
  t.setUTCSeconds(0, 0);
  for (let i = 0; i < 366 * 24 * 60; i++) {
    t.setUTCMinutes(t.getUTCMinutes() + 1);
    if (
      schedule.minutes.includes(t.getUTCMinutes()) &&
      schedule.hours.includes(t.getUTCHours()) &&
      schedule.days.includes(t.getUTCDate()) &&
      schedule.months.includes(t.getUTCMonth() + 1) &&
      schedule.weekdays.includes(t.getUTCDay())
    ) {
      return t;
    }
  }
  throw new CronError("no match within 5 years");
}
