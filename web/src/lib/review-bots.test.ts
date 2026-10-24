import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  botLogin,
  botNames,
  botSlug,
  botView,
  KNOWN_BOTS,
  matchBots,
  meant,
  sameBots,
  sized,
} from "./review-bots";

describe("botLogin", () => {
  it("reads an app however people name it", () => {
    for (const name of [
      "renovate",
      " Renovate ",
      "renovate[bot]",
      "@renovate[bot]",
      "RENOVATE[BOT]",
      "https://github.com/apps/renovate",
      "github.com/apps/renovate/",
      "https://www.github.com/apps/renovate?tab=about",
      "http://github.com/apps/renovate#x",
    ]) {
      expect(botLogin(name), name).toBe("renovate[bot]");
    }
    expect(botLogin("pre-commit-ci")).toBe("pre-commit-ci[bot]");
  });

  it("refuses what can't be an app", () => {
    for (const bad of [
      "",
      "[bot]",
      "https://github.com/renovate",
      "https://github.com/apps/",
      "-renovate",
      "reno vate",
      "renovate[bot][bot]",
      "acme/web",
      "évil",
      "a".repeat(65),
    ]) {
      expect(botLogin(bad), bad).toBeNull();
    }
  });
});

describe("the apps the Reviews tab knows", () => {
  it("are named as their bots are, once each", () => {
    const logins = KNOWN_BOTS.map((b) => b.login);
    expect(new Set(logins).size).toBe(logins.length);
    for (const b of KNOWN_BOTS) expect(botLogin(b.login)).toBe(b.login);
    expect(logins.slice(0, 2)).toEqual(["dependabot[bot]", "renovate[bot]"]);
  });
});

const seen: G.ReviewBot = {
  login: "renovate[bot]",
  url: "https://github.com/apps/renovate",
  avatar_url: "https://avatars.githubusercontent.com/in/2740?v=4",
  pull_requests: 3,
};

describe("botView", () => {
  it("names known apps and shows their logos", () => {
    expect(botView("dependabot[bot]")).toEqual({
      login: "dependabot[bot]",
      name: "Dependabot",
      logo: "https://avatars.githubusercontent.com/in/29110",
      about: "Dependency and security updates",
      owner: undefined,
      url: "https://github.com/apps/dependabot",
      seen: undefined,
    });
  });

  it("takes what GitHub said over what it knows", () => {
    const v = botView("renovate[bot]", seen, {
      login: "renovate[bot]",
      name: "Mend Renovate",
      owner: "mend",
      url: "https://github.com/apps/renovate",
      avatar_url: "https://avatars.githubusercontent.com/in/2740",
      pull_requests: 0,
    });
    expect(v.name).toBe("Mend Renovate");
    expect(v.owner).toBe("mend");
    expect(v.logo).toBe("https://avatars.githubusercontent.com/in/2740");
    expect(v.seen).toBe(seen);
  });

  it("names other apps by their slug, without a logo nobody gave", () => {
    const v = botView("acme-releaser[bot]");
    expect(v.name).toBe("acme-releaser");
    expect(v.logo).toBeUndefined();
    expect(v.url).toBe("https://github.com/apps/acme-releaser");
    expect(botView("acme-releaser[bot]", { ...seen, login: "acme-releaser[bot]" }).logo).toBe(
      seen.avatar_url,
    );
  });
});

describe("matchBots", () => {
  const views = ["dependabot[bot]", "renovate[bot]", "github-actions[bot]", "pre-commit-ci[bot]"].map((l) =>
    botView(l),
  );
  const logins = (q: string) => matchBots(q, views).map((v) => v.login);

  it("finds apps by name or bot, those that start with it first", () => {
    expect(logins("ren")).toEqual(["renovate[bot]"]);
    expect(logins("GitHub Actions")).toEqual(["github-actions[bot]"]);
    expect(logins("github-actions[bot]")).toEqual(["github-actions[bot]"]);
    expect(logins("precommit")).toEqual(["pre-commit-ci[bot]"]);
    expect(logins("@dependabot")).toEqual(["dependabot[bot]"]);
    expect(logins("bot")).toEqual(["dependabot[bot]"]);
    expect(logins("a")).toEqual(["github-actions[bot]", "dependabot[bot]", "renovate[bot]"]);
  });

  it("keeps them all for nothing typed, and none for no match", () => {
    expect(logins(" ")).toHaveLength(4);
    expect(logins("zz")).toEqual([]);
  });
});

describe("meant", () => {
  it("guesses a slip of a letter or two", () => {
    const logins = KNOWN_BOTS.map((b) => b.login);
    expect(meant("renovat[bot]", logins)).toBe("renovate[bot]");
    expect(meant("dependabto[bot]", logins)).toBe("dependabot[bot]");
    expect(meant("renovate[bot]", logins)).toBeUndefined();
    expect(meant("acme-releaser[bot]", logins)).toBeUndefined();
    expect(meant("ren[bot]", logins)).toBeUndefined();
  });
});

describe("the rest", () => {
  it("compares lists as sets", () => {
    expect(sameBots(["a[bot]", "b[bot]"], ["B[bot]", "a[bot]"])).toBe(true);
    expect(sameBots(["a[bot]"], ["a[bot]", "b[bot]"])).toBe(false);
    expect(sameBots([], [])).toBe(true);
  });

  it("sizes avatars", () => {
    expect(sized("https://avatars.githubusercontent.com/in/2740?v=4", 48)).toBe(
      "https://avatars.githubusercontent.com/in/2740?v=4&s=48",
    );
    expect(sized("not a url", 48)).toBe("not a url");
  });

  it("lists names", () => {
    expect(botNames([])).toBe("");
    expect(botNames(["Dependabot"])).toBe("Dependabot");
    expect(botNames(["Dependabot", "Renovate"])).toBe("Dependabot and Renovate");
    expect(botNames(["Dependabot", "Renovate", "Mergify"])).toBe("Dependabot, Renovate and Mergify");
    expect(botNames(["Dependabot", "Renovate", "Mergify", "Imgbot"])).toBe(
      "Dependabot, Renovate and 2 other apps",
    );
    expect(botSlug("renovate[bot]")).toBe("renovate");
  });
});
