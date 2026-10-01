export type Handler = (payload: unknown) => void;

/** Named events, and the handlers listening for them. */
export class EventBus {
  private handlers = new Map<string, Handler[]>();

  on(event: string, handler: Handler): void {
    const list = this.handlers.get(event) ?? [];
    list.push(handler);
    this.handlers.set(event, list);
  }

  off(event: string, handler: Handler): void {
    const list = this.handlers.get(event) ?? [];
    this.handlers.set(
      event,
      list.filter((h) => h !== handler),
    );
  }

  emit(event: string, payload?: unknown): void {
    for (const handler of this.handlers.get(event) ?? []) {
      handler(payload);
    }
  }
}
