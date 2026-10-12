// Copies the compiled WASM next to the compiled TypeScript. Plain Node so the
// build works the same in cmd, PowerShell and POSIX shells.
import { copyFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const src = new URL("../../target/wasm32-unknown-unknown/release/shrinkfit.wasm", root);
const dist = new URL("dist/", root);
mkdirSync(fileURLToPath(dist), { recursive: true });
copyFileSync(fileURLToPath(src), fileURLToPath(new URL("shrinkfit.wasm", dist)));
