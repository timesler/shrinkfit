// Run after `npm run build`. Uses only node:test, no dependencies.
import { test } from "node:test";
import assert from "node:assert/strict";
import { init, shrink, shrinkAsync, shrinkWithReport, shrinkWithReportAsync, ShrinkError } from "../dist/index.js";

await init();

test("keeps schema and fits the budget", () => {
  const data = { status: "ok", total: 1000, items: [...Array(1000).keys()].map((i) => ({ id: i, name: `n${i}` })) };
  const { text, report } = shrinkWithReport(data, 300);
  assert.ok([...text].length <= 300);
  assert.equal(report.fits, true);
  const parsed = JSON.parse(text);
  assert.equal(parsed.total, 1000);
  assert.match(text, /items omitted/);
});

test("string input and object input agree", () => {
  const data = { a: "x".repeat(5000), b: [1, 2, 3] };
  assert.equal(shrink(data, 200), shrink(JSON.stringify(data), 200));
});

test("yaml input", () => {
  const src = "items:\n" + Array.from({ length: 300 }, (_, i) => `  - id: ${i}\n`).join("");
  const out = shrink(src, 200, { inputFormat: "yaml" });
  assert.ok([...out].length <= 200);
  assert.match(out, /items omitted/);
  assert.throws(() => shrink("a: [1", 100, { inputFormat: "yaml" }), ShrinkError);
});

test("yaml output and camelCase options", () => {
  const out = shrink({ notes: Array.from({ length: 200 }, (_, i) => `line ${i}`).join("\n") }, 400, {
    format: "yaml",
    stringSectionChars: 20,
  });
  assert.ok([...out].length <= 400);
  assert.match(out, /^notes: \|/);
});

test("errors", () => {
  assert.throws(() => shrink("{bad", 10), ShrinkError);
  assert.throws(() => shrink({}, 10, { bogus: 1 }), ShrinkError);
});

test("jev option is accepted but wasm cannot make requests, so built-in scoring is used", () => {
  const rows = Array.from({ length: 30 }, (_, i) => ({ id: `ID-${i}`, alpha: `alpha text ${i} `.repeat(6), beta: `beta text ${i} `.repeat(6) }));
  const opts = { arrayStrategy: "condense", indent: 0 };
  const plain = shrinkWithReport({ results: rows }, 1500, opts);
  const withJev = shrinkWithReport({ results: rows }, 1500, { ...opts, jev: { apiKey: "k", tool: "search", timeoutMs: 500 } });
  assert.equal(withJev.text, plain.text);
  assert.match(withJev.report.model_scoring, /unavailable/);
  assert.throws(() => shrinkWithReport({ results: rows }, 1500, { ...opts, jev: { nope: 1 } }), ShrinkError);
});

const jevRows = () => Array.from({ length: 30 }, (_, i) => ({ id: `ID-${i}`, alpha: `alpha text ${i} `.repeat(6), beta: `beta text ${i} `.repeat(6) }));
const jevOpts = { arrayStrategy: "condense", indent: 0, fieldScoreWeight: 1 };
// A stand-in for the Decisions API: P(yes) = 1 for fields ending in `favour`, else 0.
const fakeFetch = (favour, log = []) => async (url, init) => {
  const body = JSON.parse(init.body);
  log.push({ url, init, body });
  const answers = Object.fromEntries(
    Object.entries(body.questions).map(([k, q]) => [k, { type: "noul", noul: q.instructions.split("`")[1].endsWith(favour) ? 1 : 0 }]),
  );
  return { ok: true, status: 200, text: async () => JSON.stringify({ answers }) };
};

test("async jev: the core builds the request, fetch sends it, the core blends the reply", async () => {
  const log = [];
  const options = { ...jevOpts, jev: { apiKey: "test-key", tool: "search", baseUrl: "https://example.test/api", fetch: fakeFetch("beta", log) } };
  const r = await shrinkWithReportAsync({ results: jevRows() }, 1500, options);
  assert.deepEqual(Object.keys(JSON.parse(r.text).results[0]), ["beta"]);
  assert.match(r.report.model_scoring, /^scored 3 fields/);
  assert.equal(log.length, 1);
  assert.equal(log[0].url, "https://example.test/api/alpha/decisions");
  assert.equal(log[0].init.headers.Authorization, "Bearer test-key");
  assert.equal(log[0].body.model, "typesafe/jev-1.13");
  const flipped = await shrinkAsync({ results: jevRows() }, 1500, { ...options, jev: { ...options.jev, fetch: fakeFetch("alpha") } });
  assert.deepEqual(Object.keys(JSON.parse(flipped).results[0]), ["alpha"]);
});

test("async jev: failures fall back to the built-in scoring and say why", async () => {
  const withFetch = (fetch) => ({ ...jevOpts, jev: { apiKey: "k", fetch } });
  const plain = shrinkWithReport({ results: jevRows() }, 1500, jevOpts);
  const status = await shrinkWithReportAsync(
    { results: jevRows() },
    1500,
    withFetch(async () => ({ ok: false, status: 402, text: async () => "" })),
  );
  assert.equal(status.text, plain.text);
  assert.match(status.report.model_scoring, /HTTP 402/);
  const down = await shrinkWithReportAsync(
    { results: jevRows() },
    1500,
    withFetch(async () => {
      throw new TypeError("fetch failed");
    }),
  );
  assert.equal(down.text, plain.text);
  assert.match(down.report.model_scoring, /the request failed/);
  const keyless = await shrinkWithReportAsync({ results: jevRows() }, 1500, { ...jevOpts, jev: { fetch: fakeFetch("beta") } });
  if (!process.env.OPENROUTER_API_KEY) assert.match(keyless.report.model_scoring, /no OpenRouter API key/);
  // Nothing to cut, nothing sent.
  let calls = 0;
  await shrinkAsync({ a: 1 }, 1000, withFetch(async () => { calls++; return { ok: true, status: 200, text: async () => "{}" }; }));
  assert.equal(calls, 0);
});

test("async jev: the global fetch works against a real HTTP server, and a slow one times out", async () => {
  const { createServer } = await import("node:http");
  const seen = [];
  const server = createServer((req, res) => {
    let raw = "";
    req.on("data", (c) => (raw += c));
    req.on("end", () => {
      const body = JSON.parse(raw);
      seen.push({ url: req.url, auth: req.headers.authorization });
      const answers = Object.fromEntries(
        Object.entries(body.questions).map(([k, q]) => [k, { type: "noul", noul: q.instructions.split("`")[1].endsWith("beta") ? 1 : 0 }]),
      );
      const reply = () => res.end(JSON.stringify({ answers }));
      if (req.headers["x-slow"] || body.model === "slow") setTimeout(reply, 600);
      else reply();
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  try {
    const baseUrl = `http://127.0.0.1:${server.address().port}/api`;
    const r = await shrinkWithReportAsync({ results: jevRows() }, 1500, { ...jevOpts, jev: { apiKey: "real-key", baseUrl } });
    assert.deepEqual(Object.keys(JSON.parse(r.text).results[0]), ["beta"]);
    assert.deepEqual(seen, [{ url: "/api/alpha/decisions", auth: "Bearer real-key" }]);
    const slow = await shrinkWithReportAsync({ results: jevRows() }, 1500, { ...jevOpts, jev: { apiKey: "k", baseUrl, model: "slow", timeoutMs: 150 } });
    assert.match(slow.report.model_scoring, /timed out/);
  } finally {
    server.close();
  }
});
