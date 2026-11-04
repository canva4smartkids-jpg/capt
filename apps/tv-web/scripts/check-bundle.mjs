// Verifies the production bundle can run on 2018-era TV browsers (Chromium 53 on webOS 4,
// Chromium 56 on Tizen 4): ES2015 syntax only, a classic (non-module) script, no `crossorigin`
// attributes (they break loading from file://), and none of the newer runtime APIs we avoid.
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import * as acorn from "acorn";

const dist = new URL("../dist/", import.meta.url).pathname;
const html = readFileSync(join(dist, "index.html"), "utf8");
const problems = [];
if (/type="module"/.test(html)) problems.push("index.html still loads a module script");
if (/crossorigin/.test(html)) problems.push("index.html contains crossorigin attributes");
if (!/<script defer src="\.\/assets\/app\.js"><\/script>/.test(html)) problems.push("index.html does not load ./assets/app.js as a deferred classic script");
if (!/<link rel="stylesheet" href="\.\/assets\/[^"]+\.css">/.test(html)) problems.push("index.html does not link the stylesheet relatively");

const assets = readdirSync(join(dist, "assets"));
for (const f of assets.filter((f) => f.endsWith(".js"))) {
  const code = readFileSync(join(dist, "assets", f), "utf8");
  try {
    acorn.parse(code, { ecmaVersion: 2015, sourceType: "script" });
  } catch (e) {
    const pos = e.pos ?? 0;
    problems.push(`${f} is not ES2015: ${e.message} near: ${code.slice(Math.max(0, pos - 60), pos + 60)}`);
  }
  for (const api of ["Object.entries(", "Object.values(", "Object.fromEntries(", ".padStart(", ".padEnd(", ".flatMap(", "structuredClone(", "AbortController", "globalThis.fetch", "queueMicrotask("]) {
    if (code.includes(api)) problems.push(`${f} uses ${api} (unavailable on 2018 TV browsers)`);
  }
}
if (problems.length) {
  console.error("TV bundle check failed:\n - " + problems.join("\n - "));
  process.exit(1);
}
console.log(`TV bundle check passed (${assets.join(", ")}): ES2015 syntax, classic script, file:// safe.`);
