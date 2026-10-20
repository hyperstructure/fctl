import { describe, expect, it } from "vitest";
import { installCommand } from "./install";

describe("installCommand", () => {
  it("takes the installer from the site beside an app at app.<domain>", () => {
    expect(installCommand("app.fctl.sh")).toBe("curl -fsSL https://fctl.sh/install | sh");
    expect(installCommand("app.acme.dev")).toBe("curl -fsSL https://acme.dev/install | sh");
  });

  it("takes fctl.sh's elsewhere, since an instance serves none", () => {
    expect(installCommand("fctl.acme.dev")).toBe("curl -fsSL https://fctl.sh/install | sh");
    expect(installCommand("localhost")).toBe("curl -fsSL https://fctl.sh/install | sh");
  });
});
