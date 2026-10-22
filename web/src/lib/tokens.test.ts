import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { describeScope, example, formScope, NEW_TOKEN, ready, tokenRequest, words } from "./tokens";

const place = (id: string, name?: string, slug?: string): G.TokenPlace => ({ id, name, slug });

describe("access tokens", () => {
  it("says what a token does in a sentence", () => {
    expect(describeScope(undefined)).toBe("Does everything you can, as you do when signed in.");
    expect(
      describeScope({
        access: "run",
        places: { workspaces: [], projects: [place("prj_1", "Web")] },
        github_access: "comment",
      }),
    ).toBe("Runs sessions in Web; their agents review and comment on GitHub, and push nothing.");
    expect(
      describeScope({
        access: "full",
        places: {
          workspaces: [place("wsp_1", "Acme")],
          projects: [place("prj_1", "Web"), place("prj_2", "API")],
        },
      }),
    ).toBe("Does everything you can in Web, API and every project of Acme.");
    // A token that reads starts no session, so what agents may do doesn't come into it.
    expect(describeScope({ access: "read", github_access: "read" })).toBe("Reads all your projects.");
    expect(describeScope({ access: "run", places: { workspaces: [], projects: [place("prj_9")] } })).toBe(
      "Runs sessions in a project that's gone.",
    );
    expect(words(["a", "b", "c"])).toBe("a, b and c");
  });

  it("asks for the limits the form sets", () => {
    expect(tokenRequest({ ...NEW_TOKEN, name: " PR reviews " })).toEqual({
      name: "PR reviews",
      access: "run",
      expires_in_days: 90,
    });
    expect(
      tokenRequest({
        name: "Reviews",
        access: "run",
        places: { workspaces: ["wsp_1"], projects: ["prj_1"] },
        github: "comment",
        expires: "never",
      }),
    ).toEqual({
      name: "Reviews",
      access: "run",
      workspaces: ["wsp_1"],
      projects: ["prj_1"],
      github_access: "comment",
    });
    // A token that reads starts nothing, so it asks for nothing about agents.
    expect(
      tokenRequest({ ...NEW_TOKEN, name: "Dashboard", access: "read", github: "read" }),
    ).not.toHaveProperty("github_access");
  });

  it("holds agents to less only in a token that runs sessions", () => {
    // One that does everything could change what its projects' agents do.
    expect(
      tokenRequest({ ...NEW_TOKEN, name: "Script", access: "full", github: "comment" }),
    ).not.toHaveProperty("github_access");
    const names = { workspaces: new Map(), projects: new Map([["prj_1", "Web"]]) };
    expect(
      formScope(
        { ...NEW_TOKEN, access: "full", github: "comment", places: { workspaces: [], projects: ["prj_1"] } },
        names,
      )?.github_access,
    ).toBeUndefined();
    expect(formScope({ ...NEW_TOKEN, access: "full", github: "comment" }, names)).toBeUndefined();
  });

  it("needs a name, and a project when it names some", () => {
    expect(ready(NEW_TOKEN)).toBe(false);
    expect(ready({ ...NEW_TOKEN, name: "x" })).toBe(true);
    expect(ready({ ...NEW_TOKEN, name: "x", places: { workspaces: [], projects: [] } })).toBe(false);
  });

  it("says what the form makes before it does", () => {
    const names = { workspaces: new Map([["wsp_1", "Acme"]]), projects: new Map([["prj_1", "Web"]]) };
    expect(formScope({ ...NEW_TOKEN, access: "full" }, names)).toBeUndefined();
    expect(
      describeScope(
        formScope(
          { ...NEW_TOKEN, places: { workspaces: [], projects: ["prj_1"] }, github: "comment" },
          names,
        ),
      ),
    ).toBe("Runs sessions in Web; their agents review and comment on GitHub, and push nothing.");
  });

  it("shows how to use a new token", () => {
    const reviews: G.TokenScope = {
      access: "run",
      places: { workspaces: [], projects: [place("prj_1", "Web", "web")] },
      github_access: "comment",
    };
    expect(example(reviews, "fctl_pat_x")).toBe(
      'FCTL_TOKEN=fctl_pat_x fctl run -p web --github comment -d "Review pull request #12"',
    );
    expect(example({ access: "read" }, "fctl_pat_x")).toBe("FCTL_TOKEN=fctl_pat_x fctl sessions --json");
    expect(example(undefined, "fctl_pat_x")).toBe(
      'FCTL_TOKEN=fctl_pat_x fctl run -d "Fix the flaky login test"',
    );
  });
});
