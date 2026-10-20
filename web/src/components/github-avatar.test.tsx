import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import headers from "../../public/_headers?raw";
import { GithubAvatar, githubAvatarUrl } from "./github-avatar";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** Where the app's Content Security Policy lets images load from. */
function imageSources(): string[] {
  const policy = headers.match(/Content-Security-Policy:(.*)/)?.[1] ?? "";
  const directive = policy
    .split(";")
    .map((d) => d.trim().split(/\s+/))
    .find((d) => d[0] === "img-src");
  return directive?.slice(1) ?? [];
}

describe("githubAvatarUrl", () => {
  it("loads from a host the Content Security Policy allows", () => {
    const url = githubAvatarUrl("hyperstructure", 64);
    expect(url).toBe("https://avatars.githubusercontent.com/hyperstructure?s=64");
    expect(imageSources()).toContain(new URL(url).origin);
  });
});

describe("GithubAvatar", () => {
  it("shows the GitHub mark when the avatar doesn't load", () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    act(() => root.render(<GithubAvatar account="hyperstructure" />));
    const img = container.querySelector("img");
    expect(img?.getAttribute("src")).toBe("https://avatars.githubusercontent.com/hyperstructure?s=64");

    act(() => img?.dispatchEvent(new Event("error")));
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("svg")).not.toBeNull();
    act(() => root.unmount());
    container.remove();
  });
});
