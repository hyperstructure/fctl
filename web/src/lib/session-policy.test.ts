import { describe, expect, it } from "vitest";
import {
  browserIdleChoices,
  browserMaxChoices,
  consequences,
  DEFAULT_POLICY,
  describe as describePolicy,
  devicesNote,
  minutes,
  tokenMaxChoices,
} from "./session-policy";

describe("session policies", () => {
  it("read as the CLI says them", () => {
    expect(describePolicy(DEFAULT_POLICY)).toEqual([
      "Browsers stay signed in while they're used, until 30 days unused.",
      "CLIs stay signed in while they're used, until 180 days unused.",
      "Everyone makes access tokens, each lasting as long as its maker chooses.",
    ]);
    expect(
      describePolicy({
        browser_max_hours: 12,
        browser_idle_minutes: 60,
        cli_max_days: 30,
        access_token_max_days: 365,
        member_access_tokens: false,
      }),
    ).toEqual([
      "Browsers sign in again after 12 hours, and after an hour unused.",
      "CLIs sign in again after 30 days, and after 180 days unused.",
      "Only admins make access tokens, each lasting a year at most.",
    ]);
    expect([1, 15, 60, 90, 1440, 10080, 43200].map(minutes)).toEqual([
      "a minute",
      "15 minutes",
      "an hour",
      "90 minutes",
      "a day",
      "a week",
      "30 days",
    ]);
  });

  it("say what a change ends, as the CLI does", () => {
    expect(consequences({ browsers: 14, clis: 1, access_tokens: 5, member_tokens: 3, you: true })).toEqual([
      "Saving signs out 14 browsers and 1 CLI at once, yours among them: their people sign in again.",
      "It ends 5 access tokens, 3 of them members': scripts and CI that use them need new ones.",
    ]);
    expect(consequences({ browsers: 0, clis: 0, access_tokens: 0, member_tokens: 0, you: false })).toEqual([
      "Nobody is signed out now: sign-ins end as it says from here on.",
    ]);
    expect(consequences({ browsers: 0, clis: 0, access_tokens: 0, member_tokens: 0, you: true })).toEqual([
      "Saving signs you out at once: you sign in again.",
    ]);
  });

  it("offer each limit's choices, the saved one among them", () => {
    expect(browserMaxChoices().map((o) => o.label)).toEqual([
      "8 hours",
      "12 hours",
      "A day",
      "A week",
      "30 days",
      "While it's used",
    ]);
    // Set with the CLI to something the page doesn't offer.
    expect(browserMaxChoices(36).map((o) => o.label)).toContain("36 hours");
    expect(browserIdleChoices().find((o) => o.value === "43200")?.description).toBe("The default");
    expect(tokenMaxChoices(60).map((o) => o.value)).toEqual(["7", "30", "60", "90", "365", "0"]);
  });

  it("tell people in Settings how long their devices stay signed in", () => {
    expect(devicesNote(DEFAULT_POLICY)).toBeUndefined();
    expect(
      devicesNote({ ...DEFAULT_POLICY, browser_max_hours: 12, browser_idle_minutes: 60, cli_max_days: 30 }),
    ).toBe(
      "This instance signs out browsers 12 hours after they sign in, browsers unused for an hour and CLIs 30 days after they sign in.",
    );
  });
});
