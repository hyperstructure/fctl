import { describe, expect, it } from "vitest";
import { firstNames, nameParts } from "./name-fields";

describe("names", () => {
  it("uses the stored first names in full", () => {
    const user = { name: "Carl Oscar Aaro", given_name: "Carl Oscar", family_name: "Aaro" };
    expect(firstNames(user)).toBe("Carl Oscar");
    expect(nameParts(user)).toEqual({ given: "Carl Oscar", family: "Aaro" });
  });

  it("splits older accounts at the last word", () => {
    expect(nameParts({ name: "Carl Oscar Aaro" })).toEqual({ given: "Carl Oscar", family: "Aaro" });
    expect(firstNames({ name: "Ada Lovelace" })).toBe("Ada");
    expect(nameParts({ name: "Cher" })).toEqual({ given: "Cher", family: "" });
  });

  it("keeps a first name without a last name", () => {
    expect(nameParts({ name: "Cher", given_name: "Cher" })).toEqual({ given: "Cher", family: "" });
  });
});
