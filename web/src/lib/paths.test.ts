import { describe, expect, it } from "vitest";
import { appPath, textLink } from "./paths";

describe("a path to go on to", () => {
  it("stays in the app", () => {
    for (const p of ["/", "/settings", "/s/ses_1?x=1#y", "/invite", "/%5Cx"]) expect(appPath(p)).toBe(p);
  });

  it("goes home instead of leaving it", () => {
    for (const p of [
      undefined,
      "",
      "settings",
      "//evil.example",
      "/\\evil.example",
      "/\t/evil.example",
      "/\n/evil.example",
      "/ /evil.example",
      "/　/evil.example",
      "https://evil.example",
      "javascript:alert(1)",
    ])
      expect(appPath(p)).toBe("/");
  });
});

describe("a link in someone's text", () => {
  it("opens web pages in a new tab and keeps app pages in the app", () => {
    expect(textLink("https://github.com/acme/app/pull/1")?.kind).toBe("web");
    expect(textLink("HTTPS://example.com")?.kind).toBe("web");
    expect(textLink("mailto:ada@example.com")?.kind).toBe("mail");
    expect(textLink("#changes")?.kind).toBe("app");
    expect(textLink("/s/ses_1")?.kind).toBe("app");
    expect(textLink("/p/playground?tab=memory")?.kind).toBe("app");
  });

  it("shows the rest as text", () => {
    for (const href of [
      "//evil.example/login",
      "/\\evil.example/login",
      "/\t/evil.example",
      "javascript:alert(1)",
      "data:text/html,hi",
      "README.md",
      "/device?code=BCDF-GHJK",
      "/invite#token=fctl_inv_x",
      "/reset#token=fctl_rst_x",
      "/setup#token=x",
      "/Login?next=/",
      "/two-factor",
    ])
      expect(textLink(href), href).toBeUndefined();
    expect(textLink("/devices")?.kind).toBe("app");
  });
});
