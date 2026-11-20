import { describe, expect, it } from "vitest";
import { da } from "./da";
import { en } from "./en";

function keys(value: object, prefix = ""): string[] {
  return Object.entries(value).flatMap(([key, child]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof child === "object" && child !== null
      ? keys(child, path)
      : [path];
  });
}

describe("i18n", () => {
  it("har samme nøgler på dansk og engelsk", () => {
    expect(keys(en).sort()).toEqual(keys(da).sort());
  });
});
