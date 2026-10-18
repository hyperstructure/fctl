import { describe, expect, it } from "vitest";
import { redact, safeAddress } from "./redact";

describe("redact", () => {
  it("hides fctl's tokens of every kind", () => {
    for (const kind of ["ws", "cli", "pat", "rt", "inv", "rst", "dc", "n"]) {
      const token = `fctl_${kind}_4f9Qx7LmZ2pR8sT1vW3yB5nC6dE0gH9jK2lM4oP6qS8`;
      expect(redact(`got ${token} back`)).toBe("got [hidden] back");
    }
  });

  it("hides the start of a token, as a JSON parse error quotes it", () => {
    expect(redact(`Unexpected token 'f', "fctl_ws_4f"... is not valid JSON`)).toBe(
      `Unexpected token 'f', "[hidden]"... is not valid JSON`,
    );
  });

  it("hides Claude's and GitHub's tokens and JSON Web Tokens", () => {
    expect(redact("sk-ant-oat01-AbC_dEf-123456789")).toBe("[hidden]");
    expect(redact("sk-ant-ort01-AbCdEf123456789 and sk-ant-api03-x1y2z3w4v5")).toBe("[hidden] and [hidden]");
    expect(redact("ghs_16C7e42F292c6912E7710c838347Ae178B4a")).toBe("[hidden]");
    expect(redact("github_pat_11ABCDEFG0123456789_abcdef")).toBe("[hidden]");
    expect(
      redact("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"),
    ).toBe("[hidden]");
  });

  it("hides values named as secrets", () => {
    expect(redact("token=abc123&next=/")).toBe("token=[hidden]&next=/");
    expect(redact('{"password": "hunter22", "name": "Ada"}')).toBe('{"password": "[hidden]", "name": "Ada"}');
    expect(redact("Authorization: Bearer abc.def.ghi")).toBe("Authorization: Bearer [hidden]");
    expect(redact("?code=4/0AY0e-g7&state=xyz")).toBe("?code=[hidden]&state=[hidden]");
  });

  it("hides email addresses", () => {
    expect(redact('Unexpected token, "{"email":"ada@example.com"..." is not valid JSON')).not.toContain(
      "ada@example.com",
    );
  });

  it("drops a URL's user, query and fragment, and keeps a stack frame's position", () => {
    expect(redact("at open (https://app.fctl.sh/invite#token=fctl_inv_abcdefghij)")).toBe(
      "at open (https://app.fctl.sh/invite)",
    );
    expect(redact("https://x-access-token:ghs_abcdefghij@github.com/acme/lib.git")).toBe(
      "https://github.com/acme/lib.git",
    );
    expect(redact("at render (http://localhost:5173/src/routes/session.tsx?t=1697:12:5)")).toBe(
      "at render (http://localhost:5173/src/routes/session.tsx:12:5)",
    );
    expect(redact("at e (https://app.fctl.sh/assets/index-Bx1.js:1:2345)")).toBe(
      "at e (https://app.fctl.sh/assets/index-Bx1.js:1:2345)",
    );
  });

  it("leaves ordinary errors as they are", () => {
    const stack =
      "TypeError: Cannot read properties of undefined (reading 'name')\n    at Row (Row.tsx:12:5)";
    expect(redact(stack)).toBe(stack);
  });
});

describe("safeAddress", () => {
  it("keeps the page, not what travels in the query or fragment", () => {
    expect(safeAddress("https://app.fctl.sh/invite#token=fctl_inv_abc")).toBe("https://app.fctl.sh/invite");
    expect(safeAddress("https://app.fctl.sh/reset?x=1#token=fctl_rst_abc")).toBe("https://app.fctl.sh/reset");
    expect(safeAddress("https://app.fctl.sh/s/ses_01M57J6NKJK3VJ0Y8NN0208YEJ")).toBe(
      "https://app.fctl.sh/s/ses_01M57J6NKJK3VJ0Y8NN0208YEJ",
    );
  });
});
