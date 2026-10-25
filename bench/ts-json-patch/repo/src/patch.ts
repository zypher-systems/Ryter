export type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

export interface Op {
  op: "add" | "remove" | "replace" | "move" | "copy" | "test" | string;
  path: string;
  value?: Json;
  from?: string;
}

export class PatchError extends Error {
  index: number;

  constructor(message: string, index: number) {
    super(message);
    this.name = "PatchError";
    this.index = index;
  }
}

function tokens(pointer: string): string[] {
  if (pointer === "") return [];
  return pointer.slice(1).split("/");
}

/** The value a JSON Pointer names. */
export function resolve(doc: Json, pointer: string): Json {
  let node: Json = doc;
  for (const token of tokens(pointer)) {
    if (Array.isArray(node)) {
      node = node[Number(token)];
    } else if (node !== null && typeof node === "object") {
      node = node[token];
    } else {
      throw new PatchError(`path not found: ${pointer}`, -1);
    }
    if (node === undefined) throw new PatchError(`path not found: ${pointer}`, -1);
  }
  return node;
}

/** Apply `ops` to `doc`, in order. */
export function applyPatch(doc: Json, ops: Op[]): Json {
  for (const op of ops) {
    const parts = tokens(op.path);
    const last = parts.pop();
    const parent = last === undefined ? null : resolve(doc, "/" + parts.join("/"));
    if (op.op === "add" || op.op === "replace") {
      if (last === undefined) {
        doc = op.value as Json;
      } else if (Array.isArray(parent)) {
        parent[Number(last)] = op.value as Json;
      } else if (parent !== null && typeof parent === "object") {
        parent[last] = op.value as Json;
      }
    } else if (op.op === "remove") {
      if (Array.isArray(parent)) {
        parent.splice(Number(last), 1);
      } else if (parent !== null && typeof parent === "object" && last !== undefined) {
        delete parent[last];
      }
    }
  }
  return doc;
}
