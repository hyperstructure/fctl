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

  it("finds the sign-in pages however a link names them", () => {
    const app = "https://app.example.com";
    for (const href of [
      // Dot segments and encoded letters, which the browser resolves and the router decodes.
      "/x/../device?code=BCDF-GHJK",
      "/s/ses_1/../../invite#token=fctl_inv_x",
      "/%2e%2e/reset#token=fctl_rst_x",
      "/./setup#token=x",
      "/%64evice?code=BCDF-GHJK",
      "/two-factor/",
      // The app's own address, in any spelling, as a web page.
      "https://app.example.com/device?code=BCDF-GHJK",
      "HTTP://APP.EXAMPLE.COM./login?next=/",
      "https://app.example.com/a/../invite#token=fctl_inv_x",
      // The API, where links start Google and GitHub sign-ins.
      "/api/v1/github/installed?installation_id=1&setup_action=install",
      "/api/v1/auth/google/start",
      "https://app.example.com/api/v1/github/install?workspace=acme",
      "/api/v1/published/../github/connect",
    ])
      expect(textLink(href, app), href).toBeUndefined();
    // Where it really goes.
    expect(textLink("/s/ses_1/../../p/acme?tab=memory", app)).toEqual({
      href: "/p/acme?tab=memory",
      kind: "app",
    });
    expect(textLink("https://app.example.com/s/ses_1", app)?.kind).toBe("web");
    expect(textLink("https://other.example.com/device?code=BCDF-GHJK", app)?.kind).toBe("web");
    // Published screenshots' links stay links.
    expect(textLink("/api/v1/published/0123456789abcdef/shot.png", app)?.kind).toBe("app");
    expect(textLink("https://app.example.com/api/v1/published/0123456789abcdef/shot.png", app)?.kind).toBe(
      "web",
    );
  });
});
