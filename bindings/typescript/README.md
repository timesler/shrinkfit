# shrinkfit for TypeScript / JavaScript

**Fit any large JSON or YAML document into a character budget without losing its shape.**

shrinkfit cuts a document down to the size you choose and keeps what a reader needs most: every field
name, the structure, a representative sample of the data, and clear markers showing what was left out. The
output is always valid JSON or YAML. It is built for AI agents and the tools that serve them: a tool that
returns 200 KB of JSON can always answer inline, and the agent can see what it got and ask a narrower
follow-up.

A zero-dependency WebAssembly build of the Rust core (about 0.6 MB), for Node 18+, Deno and browsers.

```bash
npm install shrinkfit
```

## Quick start

```ts
import { init, shrink, shrinkWithReport } from "shrinkfit";

await init();                                  // loads shrinkfit.wasm next to this module, once

shrink(bigObject);                             // indented JSON, 10000-character budget
shrink(bigObject, 4000, { indent: 0 });        // compact one-line JSON: most information per character
shrink(yamlOrJsonText, 4000);                  // text in: format detected, same format out
shrink(bigObject, 4000, { format: "yaml" });

const { text, report } = shrinkWithReport(bigObject, 4000);
report.fits; report.items_omitted;
```

Cuts are marked in place so nothing is hidden: `"…[118 items omitted]"`, `"…": "12 keys omitted"`,
`…[+2133 chars]`.

## Lists of records

For search results, rows or issues, keep **every** record and drop the same low-value fields from all
of them, and tell shrinkfit what the call asked for:

```ts
shrink(result, 4000, {
  arrayStrategy: "condense",
  arguments: { jql: "project = OPS AND status = Done" },   // fields it mentions are favoured
  protect: ["/issues/*/key"],                               // never truncate these values
  indent: 0,
});
```

## Let a decision model choose the fields

shrinkfit can ask the hosted decision model **Jev** which fields matter, and blend its answer with the
built-in heuristic. Use the **async** functions for this, because a network reply cannot be waited for
synchronously:

```ts
import { init, shrinkWithReportAsync } from "shrinkfit";

await init();
const { text, report } = await shrinkWithReportAsync(data, 4000, {
  arrayStrategy: "condense",
  jev: { tool: "search_issues" },        // key from OPENROUTER_API_KEY, or jev: { apiKey }
});
report.model_scoring;                    // 'scored 24 fields with typesafe/jev-1.13 in 231 ms'
```

WebAssembly cannot open network connections, so shrinkfit builds the request and reads the reply, and your
runtime's `fetch` carries it in between. Pass `jev: { fetch }` to use your own (a proxy, auth middleware). It is
opt-in and best effort: if the call cannot be made, shrinkfit uses the built-in scoring and `model_scoring`
tells you why. It sends field names and a few example values to OpenRouter, so never put the API key in browser
code. In an evaluation on 238 tool responses it raised the share of essential fields kept at a 25% budget from
0.77 to 0.83. See the
[model scoring guide](https://github.com/timesler/shrinkfit/blob/main/docs/model-scoring.md).
*(Needs shrinkfit 0.3.0 or later.)*

## More

* [All options and the report](https://github.com/timesler/shrinkfit/blob/main/docs/options.md)
* [Using shrinkfit in an agent or MCP tool](https://github.com/timesler/shrinkfit/blob/main/docs/agents.md)
* [How it works](https://github.com/timesler/shrinkfit/blob/main/docs/ALGORITHM.md)
* [Project home](https://github.com/timesler/shrinkfit)

License: Apache-2.0
