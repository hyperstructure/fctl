import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { ICONS } from "./project-icon-data";
import { fittingIcon, iconDef, iconOf, sameIcon, searchIcons, suggestIcons } from "./project-icons";

describe("project icons", () => {
  it("has every icon in all three styles, with a label and a group", () => {
    const names = new Set<string>();
    for (const i of ICONS) {
      expect(names.has(i.name), i.name).toBe(false);
      names.add(i.name);
      expect(i.label).not.toBe("");
      expect(["Build", "Business", "Life"]).toContain(i.group);
      for (const style of ["outline", "duotone", "solid"] as const)
        expect(i[style].length, i.name).toBeGreaterThan(0);
    }
  });

  it("draws only shapes, with nothing that runs or loads", () => {
    const tags = new Set<string>();
    const attrs = new Set<string>();
    const walk = (els: (typeof ICONS)[number]["outline"]) => {
      for (const [tag, a, ...kids] of els) {
        tags.add(tag);
        for (const k of Object.keys(a)) attrs.add(k);
        walk(kids);
      }
    };
    for (const i of ICONS) for (const style of ["outline", "duotone", "solid"] as const) walk(i[style]);
    for (const tag of tags)
      expect([
        "g",
        "path",
        "circle",
        "ellipse",
        "rect",
        "line",
        "polyline",
        "polygon",
        "defs",
        "mask",
      ]).toContain(tag);
    for (const a of attrs) expect(a).not.toMatch(/^on|href/i);
  });

  it("suggests from the name first, then the description's emoji and words", () => {
    expect(suggestIcons({ name: "Shop", description: "🥕 The web shop: build a box" })[0]).toBe("store");
    expect(suggestIcons({ name: "sourdough", description: "🍞 Feeding reminders" })[0]).toBe("bread");
    // Nothing in the name: the description's emoji.
    expect(suggestIcons({ name: "milkfloat", description: "🚐 A vehicle-routing solver" })[0]).toBe(
      "delivery-van",
    );
    expect(suggestIcons({ name: "Infra", description: "Terraform for AWS" })[0]).toBe("cloud");
    // A name that names an icon.
    expect(suggestIcons({ name: "Rocket" })).toContain("rocket");
  });

  it("suggests messaging, library and animal icons from what a service is built with", () => {
    const broker = suggestIcons(
      {
        name: "dynamoxy",
        description:
          "SNS + SQS messaging for Python with a Rust core. Consume with routing, retries, DLQs, heartbeats and graceful shutdown.",
      },
      12,
    );
    expect(broker[0]).toBe("pub-sub");
    expect(broker).toEqual(expect.arrayContaining(["message-queue", "snake", "crab", "activity"]));
    const lib = suggestIcons(
      {
        name: "tomodachi",
        description:
          "Microservice lib for Python and asyncio: HTTP, AWS SNS+SQS, RabbitMQ / AMQP, middlewares.",
      },
      12,
    );
    expect(lib[0]).toBe("topology-mesh");
    expect(lib).toEqual(
      expect.arrayContaining(["pub-sub", "message-queue", "library", "snake", "integration-plug"]),
    );
    // Words that only sound like messaging don't suggest it.
    expect(suggestIcons({ name: "Buyer", description: "A consumer mobile app" })[0]).toBe("smartphone");
    expect(suggestIcons({ name: "Leads", description: "Mortgage broker CRM" }, 12)).not.toContain(
      "message-queue",
    );
    expect(suggestIcons({ name: "Radio", description: "Our podcast broadcasts" })[0]).toBe("music-note");
    // An animal's emoji.
    expect(suggestIcons({ name: "zoo", description: "🦊 Foxes" })[0]).toBe("fox");
  });

  it("suggests brewing, factory, voting and cooking icons", () => {
    // Homebrew's mark is a beer mug.
    expect(suggestIcons({ name: "homebrew", description: "Homebrew tap of fctl CLI." })[0]).toBe("beer");
    expect(suggestIcons({ name: "Formulae", description: "Our Homebrew tap" })).toEqual(
      expect.arrayContaining(["beer", "package-open"]),
    );
    expect(suggestIcons({ name: "Works", description: "🏭 A software factory" }).slice(0, 2)).toEqual([
      "factory",
      "manufacturing-plant",
    ]);
    // A poll in a name's last part, and what voting on decisions is about.
    expect(suggestIcons({ name: "metapoll", description: "Web app and backend for metapoll.io" })[0]).toBe(
      "ballot-box",
    );
    expect(
      suggestIcons({ name: "Atlas", description: "Ranked choice voting on decision spaces" }, 4),
    ).toEqual(expect.arrayContaining(["ballot-box", "ranking", "decision-tree"]));
    // Polling is how code waits, not voting.
    expect(suggestIcons({ name: "Sync", description: "Long polling for updates" }, 12)).not.toContain(
      "ballot-box",
    );
    // What it cooks before what kind of app it is.
    const recipes = suggestIcons({
      name: "Pantry",
      description: "A baking and cooking web app with recipes",
    });
    expect(recipes[0]).toBe("chef-hat");
    expect(recipes).toEqual(expect.arrayContaining(["whisk", "cooking-pot", "smartphone"]));
    expect(suggestIcons({ name: "Bakery", description: "🥐 Orders" })).toEqual(
      expect.arrayContaining(["bread", "croissant"]),
    );
  });

  it("falls back on a generic icon that stays the same for a project", () => {
    const one = fittingIcon({ id: "prj_1", name: "Zxq" });
    expect(fittingIcon({ id: "prj_1", name: "Zxq" })).toEqual(one);
    expect(iconDef(one.name)).toBeDefined();
    expect(one).toMatchObject({ color: "ink", style: "outline" });
  });

  it("shows a project's own icon, and the one that fits in place of one it doesn't know", () => {
    expect(iconOf({ name: "Shop", icon: { name: "rocket", color: "pink", style: "solid" } })).toEqual({
      name: "rocket",
      color: "pink",
      style: "solid",
      chosen: true,
    });
    // From an older version of the app, or a newer one with more icons.
    expect(iconOf({ name: "Shop", icon: { name: "unicorn-drone", color: "blue" } })).toEqual({
      name: "store",
      color: "blue",
      style: "outline",
      chosen: true,
    });
    expect(iconOf({ name: "Shop" })).toMatchObject({ name: "store", chosen: false });
  });

  it("draws a color or a style it doesn't know as ink lines", () => {
    // From a control plane newer than the page.
    const icon = { name: "rocket", color: "teal", style: "neon" } as unknown as G.ProjectIcon;
    expect(iconOf({ name: "Shop", icon })).toEqual({
      name: "rocket",
      color: "ink",
      style: "outline",
      chosen: true,
    });
  });

  it("finds icons by name, label and tags, the best matches first", () => {
    const names = searchIcons("server").map((i) => i.name);
    expect(names[0]).toBe("server-stack");
    expect(names).toContain("cloud-server");
    expect(searchIcons("van").map((i) => i.name)).toContain("delivery-van");
    expect(searchIcons("no such icon at all")).toEqual([]);
    // Tags of our own, beside IconMCP's.
    expect(searchIcons("fanout").map((i) => i.name)).toContain("pub-sub");
    expect(searchIcons("python")[0]?.name).toBe("snake");
    expect(searchIcons("rabbitmq").map((i) => i.name)).toEqual(
      expect.arrayContaining(["message-queue", "rabbit"]),
    );
    expect(searchIcons("beer")[0]?.name).toBe("beer");
    expect(searchIcons("homebrew").map((i) => i.name)).toEqual(
      expect.arrayContaining(["beer", "beer-bottle"]),
    );
    expect(searchIcons("software factory").map((i) => i.name)).toEqual(
      expect.arrayContaining(["factory", "manufacturing-plant", "assembly-line"]),
    );
    expect(searchIcons("ranked choice").map((i) => i.name)).toEqual(
      expect.arrayContaining(["ranking", "poll-options"]),
    );
    expect(searchIcons("vote").map((i) => i.name)).toEqual(expect.arrayContaining(["ballot-box", "poll"]));
    expect(searchIcons("recipes").map((i) => i.name)).toEqual(
      expect.arrayContaining(["chef-hat", "cooking-pot", "utensils"]),
    );
    expect(searchIcons("baking").map((i) => i.name)).toEqual(
      expect.arrayContaining(["whisk", "rolling-pin", "cupcake", "croissant"]),
    );
    expect(searchIcons("  ")).toHaveLength(ICONS.length);
  });

  it("tells whether two icons draw the same", () => {
    expect(sameIcon({ name: "cube" }, { name: "cube", color: "ink", style: "outline" })).toBe(true);
    expect(sameIcon({ name: "cube", color: "pink" }, { name: "cube" })).toBe(false);
    expect(sameIcon(undefined, undefined)).toBe(true);
  });
});
