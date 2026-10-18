import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { buildsImages, imageSource, isDevcontainer, remember, runsImages, withSource } from "./images";

const settings = (s: Partial<G.ProjectSettings>): G.ProjectSettings => ({
  auto_pull_request: true,
  remote_control: true,
  claude_ai_connectors: true,
  credit_people: true,
  credit_claude: true,
  publish_screenshots: false,
  env: {},
  ...s,
});

describe("sandbox images", () => {
  it("knows where sessions start from", () => {
    expect(imageSource(settings({}))).toBe("default");
    expect(imageSource(settings({ image: "fctl-agent:latest" }))).toBe("default");
    expect(imageSource(settings({ image: "ghcr.io/acme/agent:1" }))).toBe("image");
    expect(imageSource(settings({ image_build: { file: "Dockerfile" } }))).toBe("build");
  });

  it("switches between them, sending null for no build", () => {
    const image = withSource(settings({ image_build: { file: "Dockerfile" } }), "image");
    expect(image.image_build).toBeNull();
    expect(JSON.parse(JSON.stringify(image))).toHaveProperty("image_build", null);
    const build = withSource(settings({ image: "ghcr.io/acme/agent:1" }), "build");
    expect(build.image).toBeUndefined();
    expect(build.image_build).toEqual({ file: "" });
    const kept = withSource(settings({ image_build: { file: "x/Dockerfile", context: "x" } }), "build");
    expect(kept.image_build).toEqual({ file: "x/Dockerfile", context: "x" });
    const none = withSource(settings({ image: "ghcr.io/acme/agent:1" }), "default");
    expect(none.image).toBeUndefined();
    expect(none.image_build).toBeNull();
  });

  it("brings back what was typed when switching back", () => {
    const start = settings({ image: "ghcr.io/acme/agent:1" });
    let typed = remember(start, {});
    const away = withSource(start, "build", typed);
    typed = remember(away, typed);
    const back = withSource(away, "image", typed);
    expect(back.image).toBe("ghcr.io/acme/agent:1");
    expect(back.image_build).toBeNull();
    const built = withSource(
      back,
      "build",
      remember(back, { build: { file: "Dockerfile", context: "app" } }),
    );
    expect(built.image_build).toEqual({ file: "Dockerfile", context: "app" });
  });

  it("tells dev container configurations from Dockerfiles", () => {
    expect(isDevcontainer(".devcontainer/devcontainer.json")).toBe(true);
    expect(isDevcontainer("Dockerfile")).toBe(false);
  });

  it("says what runners run and build", () => {
    expect(runsImages(undefined)).toBe("runs any image a project chooses");
    expect(runsImages(["*"])).toBe("runs any image a project chooses");
    expect(runsImages([])).toBe("runs only its own image");
    expect(runsImages(["ghcr.io/acme/*", "ubuntu"])).toBe("runs its own image and ghcr.io/acme/*, ubuntu");
    expect(buildsImages(undefined)).toBe("builds no images");
    expect(buildsImages(["acme/*"])).toBe("builds images from acme/*");
    expect(buildsImages(["*"])).toBe("builds images from any repository");
  });
});
