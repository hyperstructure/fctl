import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { isPublishedLink, lasts, linkToken, markdown } from "./published";

const token = "7NnB2kJm4QxZ0aYc9VdR3sT1uW5eF8gH6iL2oP4qK0r";

describe("published screenshots", () => {
  it("finds the token in a link", () => {
    expect(token).toHaveLength(43);
    expect(linkToken(`https://app.fctl.sh/api/v1/published/${token}/after.png`)).toBe(token);
    expect(linkToken(`http://localhost:5173/api/v1/published/${token}`)).toBe(token);
    expect(linkToken(`https://app.fctl.sh/api/v1/published/${token}x/after.png`)).toBeUndefined();
    expect(linkToken("https://app.fctl.sh/s/ses_1")).toBeUndefined();
  });

  it("shows only this instance's links", () => {
    const app = "https://app.fctl.sh";
    expect(isPublishedLink(`${app}/api/v1/published/${token}/after.png`, app)).toBe(true);
    for (const url of [
      `https://evil.example/api/v1/published/${token}/after.png`,
      `${app}/api/v1/published/${token}x/after.png`,
      `${app}/s/ses_1#/api/v1/published/${token}/after.png`,
      `data:image/png;base64,iVBORw0KGgo=/api/v1/published/${token}/a.png`,
      "javascript:alert(1)",
      "not a link",
    ])
      expect(isPublishedLink(url, app), url).toBe(false);
    // In development the control plane answers on its own port.
    expect(
      isPublishedLink(`http://localhost:8787/api/v1/published/${token}/a.png`, "http://localhost:5173", true),
    ).toBe(true);
    expect(
      isPublishedLink(`data:text/html,/api/v1/published/${token}/a.png`, "http://localhost:5173", true),
    ).toBe(false);
  });

  it("writes Markdown that shows it", () => {
    const f: G.PublishedFile = {
      url: `https://app.fctl.sh/api/v1/published/${token}/after.png`,
      name: "after [light].png",
      media_type: "image/png",
      size: 1,
      expires_at: 0,
    };
    expect(markdown(f)).toBe(`![after light.png](${f.url})`);
  });

  it("says how long links work", () => {
    expect(lasts(1)).toBe("24 hours");
    expect(lasts(30)).toBe("30 days");
  });
});
