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

/** The part of `fetch` that model scoring needs; the global `fetch` fits, so does any wrapper around it. */
export type FetchLike = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: string; signal?: AbortSignal },
) => Promise<{ ok: boolean; status: number; text(): Promise<string> }>;

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
  /** The tool call's arguments (string, or an object whose values are used): fields they mention are boosted by `condense`. */
  arguments?: unknown;
  /** Field ranking used by `condense`: "heuristic" (default), "compact" or "key_order". */
  fieldScorer?: "heuristic" | "compact" | "key_order";
  /** Paths whose values must never be altered (for example "ari" or "content/id"; a leading slash anchors at the root and a star matches one key or array position). Kept exactly, or the result reports no-fit. */
  protect?: string[];
  /** Field scores from outside, for example from a model that has read the response: path pattern (same syntax as `protect`) to a score from 0 to 1. A field they name is scored `(1 - fieldScoreWeight) * built-in + fieldScoreWeight * external`. Used by `condense`. */
  fieldScores?: Record<string, number>;
  /** How strongly `fieldScores` pull against the built-in score, 0 to 1 (default 0.5). */
  fieldScoreWeight?: number;
  /**
   * Blend in scores from a hosted decision model (Jev via OpenRouter). Pass `true` (the key is read from
   * `OPENROUTER_API_KEY`) or `{ apiKey, tool, model, baseUrl, timeoutMs, fetch }`. The WebAssembly core builds the
   * request and blends the reply but cannot open sockets, so this takes effect in the async functions
   * (`shrinkAsync`, `shrinkWithReportAsync`), which send the request with the global `fetch`, or with `jev.fetch`
   * if you give one (a proxy, auth middleware, a runtime without global fetch, a test double). Callers never talk to
   * the model themselves. The sync functions report `model_scoring: "unavailable: ..."` and use the built-in
   * scoring. Only used with the `condense` array strategy. Never put a key in browser code.
   */
  jev?: boolean | { apiKey?: string; tool?: string; model?: string; baseUrl?: string; timeoutMs?: number; fetch?: FetchLike };
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
  /** Protected values left out because their record, item or key was omitted (check this, not just `fits`). */
  protected_omitted?: number;
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
  shrinkfit_model_request(input: number, inputLen: number, opts: number, optsLen: number, outLen: number): number;
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
    protect: "protect",
    fieldScores: "field_scores",
    fieldScoreWeight: "field_score_weight",
    jev: "jev",
    arguments: "arguments",
  };
  for (const [k, v] of Object.entries(o)) {
    if (v === undefined) continue;
    const snake = map[k as keyof ShrinkOptions];
    if (!snake) throw new ShrinkError(`unknown option: ${k}`);
    if (k === "jev" && v && typeof v === "object") {
      const j = v as Record<string, unknown>;
      const names: Record<string, string> = { apiKey: "api_key", baseUrl: "base_url", timeoutMs: "timeout_ms", tool: "tool", model: "model" };
      out[snake] = Object.fromEntries(
        Object.entries(j).filter(([n, x]) => x !== undefined && n !== "fetch").map(([n, x]) => {
          if (!names[n]) throw new ShrinkError(`unknown jev option: ${n}`);
          return [names[n], x];
        }),
      );
      continue;
    }
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

/** Call one of the string-in, envelope-out wasm exports. */
function callWasm(
  name: "shrinkfit_shrink" | "shrinkfit_model_request",
  input: Uint8Array,
  opts: Uint8Array,
): Record<string, any> {
  const w = wasm;
  if (!w) throw new ShrinkError("shrinkfit is not initialized: await init() first");
  const inPtr = put(w, input);
  const optPtr = put(w, opts);
  const lenPtr = w.shrinkfit_alloc(4); // usize is 32-bit on wasm32
  let resPtr = 0;
  let resLen = 0;
  try {
    resPtr = w[name](inPtr, input.length, optPtr, opts.length, lenPtr);
    // Memory may have grown during the call: take fresh views.
    resLen = new DataView(w.memory.buffer).getUint32(lenPtr, true);
    return JSON.parse(decoder.decode(new Uint8Array(w.memory.buffer, resPtr, resLen)));
  } finally {
    w.shrinkfit_free(inPtr, input.length);
    w.shrinkfit_free(optPtr, opts.length);
    w.shrinkfit_free(lenPtr, 4);
    if (resPtr) w.shrinkfit_free(resPtr, resLen);
  }
}

function toJson(data: unknown): string {
  const json = typeof data === "string" ? data : JSON.stringify(data);
  if (json === undefined) throw new ShrinkError("data is not JSON-serializable");
  return json;
}

/**
 * Shrink `data` to at most `maxChars` characters and report what was cut.
 * A string is treated as a serialized JSON document; anything else is
 * serialized with `JSON.stringify` first.
 */
export function shrinkWithReport(data: unknown, maxChars = 10_000, options: ShrinkOptions = {}): ShrinkResult {
  const envelope = callWasm(
    "shrinkfit_shrink",
    encoder.encode(toJson(data)),
    encoder.encode(JSON.stringify(toSnakeOptions(maxChars, options))),
  );
  if (!envelope.ok) throw new ShrinkError(envelope.error);
  return { text: envelope.text, report: envelope.report };
}

/** Like `shrinkWithReport` but returns only the text. */
export function shrink(data: unknown, maxChars = 10_000, options: ShrinkOptions = {}): string {
  return shrinkWithReport(data, maxChars, options).text;
}

function envKey(): string | undefined {
  const g = globalThis as { process?: { env?: Record<string, string | undefined> } };
  return g.process?.env?.OPENROUTER_API_KEY;
}

/**
 * Like `shrinkWithReport`, and the way to use the `jev` option: when it is set and the document has to be cut,
 * the core builds the model-scoring request, this sends it with `fetch` (a 3 s default timeout, `jev.timeoutMs`
 * to change it; `jev.fetch` replaces the global `fetch`) and hands the reply back to the core, which blends it in. Any failure falls back to the built-in
 * scoring and is described in `report.model_scoring`.
 */
export async function shrinkWithReportAsync(
  data: unknown,
  maxChars = 10_000,
  options: ShrinkOptions = {},
): Promise<ShrinkResult> {
  const snake = toSnakeOptions(maxChars, options);
  const jev = snake.jev;
  if (jev && !("jev_reply" in snake) && !("jev_error" in snake)) {
    const input = encoder.encode(toJson(data));
    const conf: Record<string, unknown> = jev === true ? {} : { ...(jev as Record<string, unknown>) };
    if (conf.api_key === undefined && envKey()) conf.api_key = envKey();
    snake.jev = conf;
    const prepared = callWasm("shrinkfit_model_request", input, encoder.encode(JSON.stringify(snake)));
    if (!prepared.ok) {
      snake.jev_error = String(prepared.error);
    } else if (prepared.request) {
      const req = prepared.request as { url: string; headers: Record<string, string>; body: string; timeout_ms: number };
      const custom = typeof options.jev === "object" ? options.jev.fetch : undefined;
      const send = custom ?? (globalThis as { fetch?: FetchLike }).fetch;
      if (!send) {
        snake.jev_error = "no fetch available to send the request";
      } else {
        try {
          const res = await send(req.url, {
            method: "POST",
            headers: req.headers,
            body: req.body,
            signal: AbortSignal.timeout(req.timeout_ms),
          });
          if (res.ok) snake.jev_reply = await res.text();
          else snake.jev_error = `the service answered HTTP ${res.status}`;
        } catch (e) {
          const name = (e as { name?: string }).name;
          snake.jev_error = name === "TimeoutError" || name === "AbortError" ? "the request timed out" : "the request failed";
        }
      }
    }
  }
  const envelope = callWasm("shrinkfit_shrink", encoder.encode(toJson(data)), encoder.encode(JSON.stringify(snake)));
  if (!envelope.ok) throw new ShrinkError(envelope.error);
  return { text: envelope.text, report: envelope.report };
}

/** Like `shrinkWithReportAsync` but returns only the text. */
export async function shrinkAsync(
  data: unknown,
  maxChars = 10_000,
  options: ShrinkOptions = {},
): Promise<string> {
  return (await shrinkWithReportAsync(data, maxChars, options)).text;
}
