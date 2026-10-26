import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Avatar } from "./misc";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { container, rerender: (next: ReactNode) => act(() => root.render(next)) };
}

const ada = { id: "usr_01ADA", name: "Ada Lovelace" };

describe("Avatar", () => {
  it("shows the picture people uploaded", () => {
    const url = "/api/v1/users/usr_01ADA/avatar?v=1761480000000";
    const { container } = render(<Avatar user={{ ...ada, avatar_url: url }} />);
    expect(container.querySelector("img")?.getAttribute("src")).toBe(url);
  });

  it("shows pictures from other sites, kept from before uploads", () => {
    const url = "https://lh3.googleusercontent.com/a/ACg8oc=s96-c";
    const { container } = render(<Avatar user={{ ...ada, avatar_url: url }} />);
    expect(container.querySelector("img")?.getAttribute("src")).toBe(url);
  });

  it("shows initials for other addresses of the app, which would act for whoever sees them", () => {
    for (const url of [
      "/api/v1/me/export",
      `${location.origin}/api/v1/me/export`,
      "/api/v1/users/usr_01ADA/avatar?v=1&then=/api/v1/me/export",
    ]) {
      const { container } = render(<Avatar user={{ ...ada, avatar_url: url }} />);
      expect(container.querySelector("img"), url).toBeNull();
      expect(container.textContent).toBe("AL");
    }
  });

  it("shows a new picture after one that didn't load", () => {
    const { container, rerender } = render(
      <Avatar user={{ ...ada, avatar_url: "https://example.com/gone.png" }} />,
    );
    act(() => {
      container.querySelector("img")?.dispatchEvent(new Event("error"));
    });
    expect(container.querySelector("img")).toBeNull();
    const url = "/api/v1/users/usr_01ADA/avatar?v=2";
    rerender(<Avatar user={{ ...ada, avatar_url: url }} />);
    expect(container.querySelector("img")?.getAttribute("src")).toBe(url);
  });
});
