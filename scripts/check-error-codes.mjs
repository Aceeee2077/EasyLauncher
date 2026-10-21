#!/usr/bin/env node
// Ensures every backend error code has a Chinese and English translation and
// that no error key is declared twice in the same language table.
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const rustRoot = "src-tauri/src";
const messagesPath = "src/i18n/messages.ts";

function walk(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    return path.endsWith(".rs") ? [path] : [];
  });
}

const codes = new Set();
for (const file of walk(rustRoot)) {
  const source = readFileSync(file, "utf8");
  for (const match of source.matchAll(/\.with_code\(\s*"([^"]+)"/g)) {
    codes.add(match[1]);
  }
  if (file.endsWith("migration.rs")) {
    for (const match of source.matchAll(/\buser\(\s*"([^"]+)"/g)) {
      codes.add(match[1]);
    }
  }
  if (file.endsWith("resources.rs")) {
    for (const match of source.matchAll(/"((?:resources|server)\.[A-Za-z][\w.]*)"/g)) {
      codes.add(match[1]);
    }
  }
  if (file.endsWith("update.rs")) {
    for (const match of source.matchAll(/\berr\(\s*"([^"]+)"/g)) {
      codes.add(match[1]);
    }
  }
}

const source = readFileSync(messagesPath, "utf8");
const split = source.indexOf("export const en");
if (split < 0) {
  console.error("error codes: could not locate the English message table");
  process.exit(1);
}

function table(part) {
  const counts = new Map();
  for (const match of part.matchAll(/"errors\.([^"]+)"\s*:/g)) {
    counts.set(match[1], (counts.get(match[1]) ?? 0) + 1);
  }
  return counts;
}

const zh = table(source.slice(0, split));
const en = table(source.slice(split));
const translated = new Set([...zh.keys(), ...en.keys()]);
const missing = [...codes].filter((code) => !translated.has(code)).sort();
const untranslatedZh = [...codes].filter((code) => !zh.has(code));
const untranslatedEn = [...codes].filter((code) => !en.has(code));
const unused = [...translated].filter((code) => !codes.has(code)).sort();
const duplicates = [
  ...[...zh].filter(([, count]) => count > 1).map(([code]) => `zh ${code}`),
  ...[...en].filter(([, count]) => count > 1).map(([code]) => `en ${code}`),
];

if (missing.length || untranslatedZh.length || untranslatedEn.length || duplicates.length) {
  if (missing.length) {
    console.error(`error codes missing from both tables (${missing.length}):`);
    for (const code of missing) console.error(`  ${code}`);
  }
  if (untranslatedZh.length) {
    console.error(`error codes missing from Chinese (${untranslatedZh.length}):`);
    for (const code of untranslatedZh) console.error(`  ${code}`);
  }
  if (untranslatedEn.length) {
    console.error(`error codes missing from English (${untranslatedEn.length}):`);
    for (const code of untranslatedEn) console.error(`  ${code}`);
  }
  if (duplicates.length) {
    console.error(`duplicate error keys (${duplicates.length}):`);
    for (const code of duplicates) console.error(`  ${code}`);
  }
  process.exit(1);
}

console.log(
  `error codes: ${codes.size} backend codes, ${translated.size} translations, ` +
    `${unused.length} unused`,
);
if (unused.length) {
  console.log(`unused translations: ${unused.join(", ")}`);
}
