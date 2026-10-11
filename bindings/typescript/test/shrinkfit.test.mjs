// Run after `npm run build`. Uses only node:test, no dependencies.
import { test } from "node:test";
import assert from "node:assert/strict";
import { init, shrink, shrinkWithReport, ShrinkError } from "../dist/index.js";

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
