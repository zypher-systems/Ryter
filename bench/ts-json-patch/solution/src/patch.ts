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

function fail(message: string, index: number): never {
  throw new PatchError(message, index);
}

function tokens(pointer: string, index: number): string[] {
  if (pointer === "") return [];
  if (!pointer.startsWith("/")) fail(`invalid pointer: ${pointer}`, index);
  return pointer
    .slice(1)
    .split("/")
    .map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
}

function clone(value: Json): Json {
  if (Array.isArray(value)) return value.map(clone);
  if (value !== null && typeof value === "object") {
    const out: { [key: string]: Json } = {};
    for (const key of Object.keys(value)) out[key] = clone(value[key]);
    return out;
  }
  return value;
}

function equal(a: Json, b: Json): boolean {
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((item, i) => equal(item, b[i]));
  }
  if (a !== null && typeof a === "object" && b !== null && typeof b === "object") {
    const ka = Object.keys(a).sort();
    const kb = Object.keys(b).sort();
    if (ka.length !== kb.length || ka.some((k, i) => k !== kb[i])) return false;
    return ka.every((k) => equal(a[k], b[k]));
  }
  return a === b;
}

function isIndex(token: string): boolean {
  return /^(0|[1-9][0-9]*)$/.test(token);
}

/** Walk to the value a pointer names; `missing` is what to do when it isn't there. */
function walk(doc: Json, pointer: string, index: number): Json {
  let node: Json = doc;
  for (const token of tokens(pointer, index)) {
    if (Array.isArray(node)) {
      if (!isIndex(token)) fail(`index out of range: ${pointer}`, index);
      const i = Number(token);
      if (i >= node.length) fail(`path not found: ${pointer}`, index);
      node = node[i];
    } else if (node !== null && typeof node === "object") {
      if (!Object.prototype.hasOwnProperty.call(node, token)) fail(`path not found: ${pointer}`, index);
      node = node[token];
    } else {
      fail(`path not found: ${pointer}`, index);
    }
  }
  return node;
}

/** The value a JSON Pointer names. */
export function resolve(doc: Json, pointer: string): Json {
  return walk(doc, pointer, -1);
}

interface Spot {
  parent: Json[] | { [key: string]: Json } | null;
  key: string;
}

/** The container and key a pointer names (parent `null` for the root). */
function spot(doc: Json, pointer: string, index: number): Spot {
  const parts = tokens(pointer, index);
  if (parts.length === 0) return { parent: null, key: "" };
  const key = parts.pop() as string;
  const parentPointer = parts.length === 0 ? "" : "/" + parts.map((t) => t.replace(/~/g, "~0").replace(/\//g, "~1")).join("/");
  const parent = walk(doc, parentPointer, index);
  if (parent === null || typeof parent !== "object") fail(`path not found: ${pointer}`, index);
  return { parent, key };
}

function add(doc: Json, pointer: string, value: Json, index: number): Json {
  const { parent, key } = spot(doc, pointer, index);
  if (parent === null) return value;
  if (Array.isArray(parent)) {
    if (key === "-") {
      parent.push(value);
    } else {
      if (!isIndex(key) || Number(key) > parent.length) fail(`index out of range: ${pointer}`, index);
      parent.splice(Number(key), 0, value);
    }
  } else {
    parent[key] = value;
  }
  return doc;
}

function remove(doc: Json, pointer: string, index: number): Json {
  const { parent, key } = spot(doc, pointer, index);
  if (parent === null) fail(`path not found: ${pointer}`, index);
  if (Array.isArray(parent)) {
    if (!isIndex(key)) fail(`index out of range: ${pointer}`, index);
    if (Number(key) >= parent.length) fail(`path not found: ${pointer}`, index);
    parent.splice(Number(key), 1);
  } else {
    if (!Object.prototype.hasOwnProperty.call(parent, key)) fail(`path not found: ${pointer}`, index);
    delete parent[key];
  }
  return doc;
}

function needValue(op: Op, index: number): Json {
  if (!("value" in op) || op.value === undefined) fail(`missing value in ${op.op}`, index);
  return op.value as Json;
}

function needFrom(op: Op, index: number): string {
  if (typeof op.from !== "string") fail(`missing from in ${op.op}`, index);
  return op.from;
}

/** Apply `ops` to `doc`, in order, as a new document. */
export function applyPatch(doc: Json, ops: Op[]): Json {
  let out = clone(doc);
  ops.forEach((op, index) => {
    switch (op.op) {
      case "add":
        out = add(out, op.path, clone(needValue(op, index)), index);
        break;
      case "remove":
        out = remove(out, op.path, index);
        break;
      case "replace": {
        const value = clone(needValue(op, index));
        walk(out, op.path, index);
        out = op.path === "" ? value : add(remove(out, op.path, index), op.path, value, index);
        break;
      }
      case "move": {
        const from = needFrom(op, index);
        tokens(op.path, index);
        if (op.path !== from && (op.path === "" ? false : op.path.startsWith(from + "/") || from === "")) {
          fail(`cannot move into own child: ${from} -> ${op.path}`, index);
        }
        const value = walk(out, from, index);
        if (op.path === from) break;
        out = remove(out, from, index);
        out = add(out, op.path, value, index);
        break;
      }
      case "copy": {
        const from = needFrom(op, index);
        tokens(op.path, index);
        const value = clone(walk(out, from, index));
        out = add(out, op.path, value, index);
        break;
      }
      case "test":
        if (!equal(walk(out, op.path, index), needValue(op, index))) fail(`test failed at ${op.path}`, index);
        break;
      default:
        fail(`unknown op: ${op.op}`, index);
    }
  });
  return out;
}
