# shrinkfit (TypeScript / JavaScript)

Shrink a JSON or YAML payload to fit a character budget while keeping as much
of its schema and structure as possible. Every cut is marked in-band, so a
reader never mistakes a sample for the whole.

A zero-dependency WebAssembly build of the Rust core, for Node 18+, Deno and
browsers.

```ts
import { init, shrink, shrinkWithReport } from "shrinkfit";

await init(); // loads shrinkfit.wasm next to this module

shrink(bigObject);                       // indented JSON, 10000-char budget
shrink(bigObject, 4000);                 // 4000-char budget
shrink(yamlOrJsonText, 4000);            // text: format auto-detected, same format out
shrink(bigObject, 4000, { format: "yaml", stringSectionChars: 60 });

const { text, report } = shrinkWithReport(bigObject, 4000);
report.fits; report.items_omitted;
```

Options (all optional): `format` (`"auto" | "json" | "yaml"`), `inputFormat`,
`indent` (default 2; 0 = compact JSON), `stringHeadChars`, `stringSectionChars`,
`stringMinSectionChars`, `keepLastItem`, `maxSamples`, `minPartialChars`.

Full documentation, the algorithm, and the Rust, Python and CLI interfaces:
<https://github.com/timesler/shrinkfit>

License: Apache-2.0
