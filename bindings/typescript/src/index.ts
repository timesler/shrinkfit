/**
 * shrinkfit: shrink JSON/YAML payloads to fit a character budget while
 * preserving their schema and structure.
 *
 * A thin, dependency-free loader for the Rust core compiled to WebAssembly.
 * It uses the same C ABI as the native bindings (no wasm-bindgen), so the
 * whole JS surface is this file.
 *
 * ```ts
 * import { init, shrink } from "shrinkfit";
 * await init();
 * shrink({ items: [...Array(1000).keys()], total: 1000 }, 80);
 * ```
 */

export interface ShrinkOptions {
  /** Output format (default "auto": same as the input). */
  format?: "auto" | "json" | "yaml";
  /** Format of string input (default "auto": JSON, else YAML). Objects are sent as JSON. */
  inputFormat?: "auto" | "json" | "yaml";
  /** Indent width (default 2). JSON: 0 = compact single-line. YAML: >= 2. */
  indent?: number;
  /** Characters of leading whole string sections to keep (default 500). */
  stringHeadChars?: number;
  /** Characters shown from each later string section (default 100). */
  stringSectionChars?: number;
  /** Smallest section preview before sections are elided (default 16). */
  stringMinSectionChars?: number;
  /** Prefer keeping the last array item when sampling (default true). */
  keepLastItem?: boolean;
  /** Shape-distinct samples when no array item fits whole (default 3). */
  maxSamples?: number;
  /** Smallest budget worth spending on a partially pruned item (default 40). */
  minPartialChars?: number;
  /** How big arrays shrink: sample whole items (default) or keep every item and drop the same low-value fields. */
  arrayStrategy?: "sample" | "condense";
  /** Field ranking used by `condense`: "heuristic" (default), "compact" or "key_order". */
  fieldScorer?: "heuristic" | "compact" | "key_order";
}

export interface Report {
  input_chars: number;
  output_chars: number;
  max_chars: number;
  pruned: boolean;
  fits: boolean;
  strings_truncated: number;
  arrays_sampled: number;
  items_omitted: number;
  objects_trimmed: number;
  keys_omitted: number;
  containers_collapsed: number;
}

export interface ShrinkResult {
  text: string;
  report: Report;
}

export class ShrinkError extends Error {
  override name = "ShrinkError";
}

interface Exports {
  memory: WebAssembly.Memory;
  shrinkfit_alloc(len: number): number;
  shrinkfit_free(ptr: number, len: number): void;
  shrinkfit_shrink(input: number, inputLen: number, opts: number, optsLen: number, outLen: number): number;
}

let wasm: Exports | null = null;

/** Sources `init` accepts: raw bytes, a compiled module, or a URL/path. */
export type WasmSource = BufferSource | WebAssembly.Module | URL | string;

/**
 * Load the WebAssembly module. Call once before `shrink`. With no argument
 * it loads `shrinkfit.wasm` next to this file (Node or browser).
 */
export async function init(source?: WasmSource): Promise<void> {
  if (wasm) return;
  const src = source ?? new URL("./shrinkfit.wasm", import.meta.url);
  let instance: WebAssembly.Instance;
  if (src instanceof WebAssembly.Module) {
    instance = await WebAssembly.instantiate(src, {});
  } else {
    const bytes = typeof src === "string" || src instanceof URL ? await readBytes(src) : src;
    instance = (await WebAssembly.instantiate(bytes, {})).instance;
  }
  wasm = instance.exports as unknown as Exports;
}

async function readBytes(src: URL | string): Promise<ArrayBuffer> {
  const url = typeof src === "string" ? new URL(src, import.meta.url) : src;
  if (url.protocol === "file:") {
    // Node: avoid fetch(file:), which Node does not support.
    // Specifier held in a variable so bundlers and tsc leave it alone.
    const fsModule = "node:fs/promises";
    const fs: { readFile(u: URL): Promise<Uint8Array> } = await import(/* @vite-ignore */ fsModule);
    const buf = await fs.readFile(url);
    return buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer;
  }
  const res = await fetch(url);
  if (!res.ok) throw new ShrinkError(`failed to load ${url}: ${res.status}`);
  return res.arrayBuffer();
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();

function toSnakeOptions(maxChars: number, o: ShrinkOptions): Record<string, unknown> {
  const out: Record<string, unknown> = { max_chars: maxChars };
  const map: Record<keyof ShrinkOptions, string> = {
    format: "format",
    inputFormat: "input_format",
    indent: "indent",
    stringHeadChars: "string_head_chars",
    stringSectionChars: "string_section_chars",
    stringMinSectionChars: "string_min_section_chars",
    keepLastItem: "keep_last_item",
    maxSamples: "max_samples",
    minPartialChars: "min_partial_chars",
    arrayStrategy: "array_strategy",
    fieldScorer: "field_scorer",
  };
  for (const [k, v] of Object.entries(o)) {
    if (v === undefined) continue;
    const snake = map[k as keyof ShrinkOptions];
    if (!snake) throw new ShrinkError(`unknown option: ${k}`);
    out[snake] = v;
  }
  return out;
}

/** Copy bytes into wasm memory; returns the pointer. */
function put(w: Exports, bytes: Uint8Array): number {
  const ptr = w.shrinkfit_alloc(bytes.length);
  new Uint8Array(w.memory.buffer, ptr, bytes.length).set(bytes);
  return ptr;
}

/**
 * Shrink `data` to at most `maxChars` characters and report what was cut.
 * A string is treated as a serialized JSON document; anything else is
 * serialized with `JSON.stringify` first.
 */
export function shrinkWithReport(data: unknown, maxChars = 10_000, options: ShrinkOptions = {}): ShrinkResult {
  const w = wasm;
  if (!w) throw new ShrinkError("shrinkfit is not initialized: await init() first");
  const json = typeof data === "string" ? data : JSON.stringify(data);
  if (json === undefined) throw new ShrinkError("data is not JSON-serializable");
  const input = encoder.encode(json);
  const opts = encoder.encode(JSON.stringify(toSnakeOptions(maxChars, options)));

  const inPtr = put(w, input);
  const optPtr = put(w, opts);
  const lenPtr = w.shrinkfit_alloc(4); // usize is 32-bit on wasm32
  let resPtr = 0;
  let resLen = 0;
  try {
    resPtr = w.shrinkfit_shrink(inPtr, input.length, optPtr, opts.length, lenPtr);
    // Memory may have grown during the call: take fresh views.
    resLen = new DataView(w.memory.buffer).getUint32(lenPtr, true);
    const envelope = JSON.parse(decoder.decode(new Uint8Array(w.memory.buffer, resPtr, resLen)));
    if (!envelope.ok) throw new ShrinkError(envelope.error);
    return { text: envelope.text, report: envelope.report };
  } finally {
    w.shrinkfit_free(inPtr, input.length);
    w.shrinkfit_free(optPtr, opts.length);
    w.shrinkfit_free(lenPtr, 4);
    if (resPtr) w.shrinkfit_free(resPtr, resLen);
  }
}

/** Like `shrinkWithReport` but returns only the text. */
export function shrink(data: unknown, maxChars = 10_000, options: ShrinkOptions = {}): string {
  return shrinkWithReport(data, maxChars, options).text;
}
