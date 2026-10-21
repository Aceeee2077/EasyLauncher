#!/usr/bin/env node
// Fails the build when the frontend outgrows its size budget. The entry chunk
// carries the app shell; each page is a separate async chunk.
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

const KB = 1024;
const budgets = {
  entry: 520 * KB,
  chunk: 80 * KB,
  total: 800 * KB,
};

const assets = "dist/assets";
const html = readFileSync("dist/index.html", "utf8");
const entryMatch = html.match(/<script[^>]+src="[^"]*\/([^"/]+\.js)"/);
if (!entryMatch) {
  console.error("bundle budget: could not find the entry script in dist/index.html");
  process.exit(1);
}
const entryName = entryMatch[1];

const files = readdirSync(assets)
  .filter((name) => name.endsWith(".js"))
  .map((name) => ({ name, size: statSync(join(assets, name)).size }))
  .sort((a, b) => b.size - a.size);

const total = files.reduce((sum, file) => sum + file.size, 0);
const failures = [];

if (!files.some((file) => file.name === entryName)) {
  failures.push(`entry chunk ${entryName} is missing from ${assets}`);
}
for (const file of files) {
  const limit = file.name === entryName ? budgets.entry : budgets.chunk;
  if (file.size > limit) {
    failures.push(
      `${file.name} is ${(file.size / KB).toFixed(1)} KiB, over its ${(limit / KB).toFixed(0)} KiB budget`,
    );
  }
}
if (total > budgets.total) {
  failures.push(
    `JavaScript totals ${(total / KB).toFixed(1)} KiB, over the ${(budgets.total / KB).toFixed(0)} KiB budget`,
  );
}

const entry = files.find((file) => file.name === entryName);
const largestAsync = files.find((file) => file.name !== entryName);
console.log(
  `bundle budget: entry ${((entry?.size ?? 0) / KB).toFixed(1)} KiB, ` +
    `largest async ${((largestAsync?.size ?? 0) / KB).toFixed(1)} KiB, ` +
    `total ${(total / KB).toFixed(1)} KiB across ${files.length} files`,
);

if (failures.length) {
  for (const failure of failures) console.error(`bundle budget: ${failure}`);
  process.exit(1);
}
