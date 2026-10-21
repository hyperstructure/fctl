import { afterEach, describe, expect, it } from "vitest";
import { forgetDraft, loadFiles, loadText, pageFiles, saveFile, saveText, setPageFiles } from "./draft";

const shot = () => ({
  key: "k1",
  name: "shot.png",
  file: new File(["x"], "shot.png", { type: "image/png" }),
});

afterEach(() => localStorage.clear());

describe("the home page's draft", () => {
  it("keeps each person's text", () => {
    saveText("usr_a", { prompt: "Fix the flaky test", project: "web", selection: [3, 8] });
    saveText("usr_b", { prompt: "Write the docs" });
    expect(loadText("usr_a")).toEqual({ prompt: "Fix the flaky test", project: "web", selection: [3, 8] });
    expect(loadText("usr_b")).toEqual({ prompt: "Write the docs", project: undefined, selection: undefined });
    expect(loadText("usr_c")).toBeUndefined();
    saveText("usr_a", undefined);
    expect(loadText("usr_a")).toBeUndefined();
  });

  it("keeps the model and effort level picked for it", () => {
    saveText("usr_a", { prompt: "Rename the button", project: "web", model: "haiku", effort: "" });
    expect(loadText("usr_a")).toEqual({
      prompt: "Rename the button",
      project: "web",
      model: "haiku",
      effort: "",
    });
  });

  it("ignores what it can't read", () => {
    localStorage.setItem("fctl:draft:usr_a", "{not json");
    expect(loadText("usr_a")).toBeUndefined();
    localStorage.setItem("fctl:draft:usr_a", JSON.stringify({ prompt: 3 }));
    expect(loadText("usr_a")).toBeUndefined();
    localStorage.setItem(
      "fctl:draft:usr_a",
      JSON.stringify({ prompt: "Hi", project: 7, selection: [1, -2], model: 4, effort: null }),
    );
    expect(loadText("usr_a")).toEqual({ prompt: "Hi", project: undefined, selection: undefined });
  });

  it("forgets one person's draft, or everyone's", async () => {
    saveText("usr_a", { prompt: "A" });
    saveText("usr_b", { prompt: "B" });
    localStorage.setItem("fctl:last-target", "web");
    setPageFiles("usr_a", [shot()]);
    setPageFiles("usr_b", [shot()]);
    await forgetDraft("usr_a");
    expect(loadText("usr_a")).toBeUndefined();
    expect(pageFiles("usr_a")).toBeUndefined();
    expect(loadText("usr_b")?.prompt).toBe("B");
    expect(pageFiles("usr_b")).toHaveLength(1);
    await forgetDraft();
    expect(loadText("usr_b")).toBeUndefined();
    expect(pageFiles("usr_b")).toBeUndefined();
    // Not a draft.
    expect(localStorage.getItem("fctl:last-target")).toBe("web");
  });

  it("keeps files only for the page where the browser stores none", async () => {
    // No IndexedDB here, as in some private windows.
    expect(await saveFile("usr_a", shot())).toBe(false);
    expect(await loadFiles("usr_a")).toEqual([]);
  });
});
