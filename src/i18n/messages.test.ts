import { describe, expect, it } from "vitest";
import { errorMessageKey, messages } from "./messages";

const zh = messages["zh-CN"];
const en = messages.en;

describe("translations", () => {
  it("keeps Chinese and English key sets identical", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(zh).sort());
  });

  it("has no empty or whitespace-only strings", () => {
    for (const [locale, table] of Object.entries(messages)) {
      for (const [key, value] of Object.entries(table)) {
        expect(value.trim(), `${locale} ${key}`).not.toBe("");
      }
    }
  });

  it("keeps the same placeholders in both languages", () => {
    const placeholders = (value: string) =>
      [...value.matchAll(/\{(\w+)\}/g)].map((match) => match[1]).sort();
    for (const key of Object.keys(zh) as (keyof typeof zh)[]) {
      expect(placeholders(en[key]), key).toEqual(placeholders(zh[key]));
    }
  });
});

describe("errorMessageKey", () => {
  it("maps known backend codes to translated keys", () => {
    const key = errorMessageKey("backup.noWorld");
    expect(key).not.toBeNull();
    if (key) expect(messages["zh-CN"][key]).toBeTruthy();
  });

  it("returns null for missing or unknown codes", () => {
    expect(errorMessageKey()).toBeNull();
    expect(errorMessageKey("")).toBeNull();
    expect(errorMessageKey("definitely.unknown")).toBeNull();
  });
});
