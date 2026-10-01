export type Handler = (payload: unknown, event: string) => void;

interface Entry {
  handler: Handler;
  once: boolean;
}

/** Named events, and the handlers listening for them. `"*"` hears them all. */
export class EventBus {
  private handlers = new Map<string, Entry[]>();

  /** Listen for `event`. Returns a function that stops listening. */
  on(event: string, handler: Handler): () => void {
    return this.add(event, { handler, once: false });
  }

  /** Listen for the next `event` only. */
  once(event: string, handler: Handler): () => void {
    return this.add(event, { handler, once: true });
  }

  off(event: string, handler: Handler): void {
    const list = this.handlers.get(event) ?? [];
    this.handlers.set(
      event,
      list.filter((entry) => entry.handler !== handler),
    );
  }

  /** Call every handler for `event`, then the wildcard's. Returns how many. */
  emit(event: string, payload?: unknown): number {
    // Copies: a handler may add or remove handlers while this runs.
    const own = [...(this.handlers.get(event) ?? [])];
    const any = event === "*" ? [] : [...(this.handlers.get("*") ?? [])];
    const errors: unknown[] = [];
    for (const [name, entries] of [
      [event, own],
      ["*", any],
    ] as const) {
      for (const entry of entries) {
        if (entry.once) this.remove(name, entry);
        try {
          entry.handler(payload, event);
        } catch (error) {
          errors.push(error);
        }
      }
    }
    if (errors.length > 0) {
      throw new AggregateError(errors, `${errors.length} handler(s) of "${event}" threw`);
    }
    return own.length + any.length;
  }

  private add(event: string, entry: Entry): () => void {
    const list = this.handlers.get(event) ?? [];
    list.push(entry);
    this.handlers.set(event, list);
    return () => this.remove(event, entry);
  }

  private remove(event: string, entry: Entry): void {
    const list = this.handlers.get(event) ?? [];
    this.handlers.set(
      event,
      list.filter((e) => e !== entry),
    );
  }
}
