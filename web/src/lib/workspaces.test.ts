import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { claudeOptions } from "./workspaces";

const cred = (c: Partial<G.ClaudeCredentialInfo> & { id: string }): G.ClaudeCredentialInfo => ({
  kind: "oauth",
  remote_control: true,
  status: "active",
  is_default: false,
  created_at: 0,
  ...c,
});

const me = (claude: G.ClaudeCredentialInfo[]) => ({ claude }) as G.Me;

describe("claudeOptions", () => {
  const work = cred({ id: "crd_work", account_email: "ada@company.example", subscription_type: "team" });
  const own = cred({ id: "crd_own", account_email: "ada@private.example", is_default: true });
  const broken = cred({ id: "crd_old", account_email: "old@company.example", status: "invalid" });

  it("offers the default first, then each working connection", () => {
    const options = claudeOptions(me([work, own, broken]), undefined);
    expect(options.map((o) => o.value)).toEqual(["default", "crd_work", "crd_own"]);
    expect(options[0]?.label).toBe("Your default (ada@private.example)");
  });

  it("keeps the chosen one when it stopped working, saying so", () => {
    const options = claudeOptions(me([work, own, broken]), "crd_old");
    expect(options.map((o) => o.value)).toEqual(["default", "crd_work", "crd_own", "crd_old"]);
    expect(options[3]?.label).toBe("old@company.example (reconnect it)");
  });

  it("names one that was removed instead of showing its id", () => {
    const options = claudeOptions(me([own]), "crd_gone", "Claude: ");
    expect(options.at(-1)).toMatchObject({ value: "crd_gone", label: "Claude: A connection you removed" });
  });
});
