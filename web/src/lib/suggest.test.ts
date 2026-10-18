import { describe, expect, it } from "vitest";
import { distance, suggest } from "./suggest";

describe("suggest", () => {
  it("counts edits", () => {
    expect(distance("kitten", "sitting")).toBe(3);
    expect(distance("", "new")).toBe(3);
  });

  it("finds a page from a typo", () => {
    expect(suggest("/setings")).toBe("/settings");
    expect(suggest("/Settings/")).toBe("/settings");
    expect(suggest("/admn")).toBe("/admin");
    expect(suggest("/devise")).toBe("/device");
  });

  it("drops what comes after a page", () => {
    expect(suggest("/settings/tokens")).toBe("/settings");
    expect(suggest("/p/my-app/sessions")).toBe("/p/my-app");
    expect(suggest("/s/ses_01M57J6NKJK3VJ0Y8NN0208YEJ/events")).toBe("/s/ses_01M57J6NKJK3VJ0Y8NN0208YEJ");
  });

  it("knows the long names of sessions and projects", () => {
    expect(suggest("/sessions/ses_01M57J6NKJK3VJ0Y8NN0208YEJ")).toBe("/s/ses_01M57J6NKJK3VJ0Y8NN0208YEJ");
    expect(suggest("/session/abc")).toBe("/s/abc");
    expect(suggest("/projects/my-app")).toBe("/p/my-app");
    expect(suggest("/sessions")).toBe("/");
    expect(suggest("/dashboard")).toBe("/");
    expect(suggest("/sign-in")).toBe("/login");
  });

  it("finds a session id anywhere", () => {
    expect(suggest("/view?id=ses_01M57J6NKJK3VJ0Y8NN0208YEJ")).toBe("/s/ses_01M57J6NKJK3VJ0Y8NN0208YEJ");
  });

  it("suggests nothing for addresses unlike any page", () => {
    expect(suggest("/")).toBeNull();
    expect(suggest("/pricing-for-cats")).toBeNull();
    expect(suggest("/p")).toBeNull();
    expect(suggest("/%E0%A4%A")).toBeNull();
  });
});
