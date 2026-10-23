import { afterEach, describe, expect, it } from "vitest";
import {
  draftId,
  forgetDraft,
  loadFiles,
  loadText,
  pageFiles,
  saveFile,
  saveText,
  setPageFiles,
} from "./draft";

const shot = () => ({
  key: "k1",
  name: "shot.png",
  file: new File(["x"], "shot.png", { type: "image/png" }),
});

afterEach(() => localStorage.clear());

describe("drafts", () => {
  it("keeps each person's text", () => {
    saveText("usr_a", { prompt: "Fix the flaky test", project: "web", selection: [3, 8] });
    saveText("usr_b", { prompt: "Write the docs" });
    expect(loadText("usr_a")).toEqual({ prompt: "Fix the flaky test", project: "web", selection: [3, 8] });
    expect(loadText("usr_b")).toEqual({ prompt: "Write the docs", project: undefined, selection: undefined });
    expect(loadText("usr_c")).toBeUndefined();
    saveText("usr_a", undefined);
    expect(loadText("usr_a")).toBeUndefined();
  });

  it("keeps the model, effort level and GitHub access picked for it", () => {
    saveText("usr_a", {
      prompt: "Rename the button",
      project: "web",
      model: "haiku",
      effort: "",
      github: "read",
    });
    expect(loadText("usr_a")).toEqual({
      prompt: "Rename the button",
      project: "web",
      model: "haiku",
      effort: "",
      github: "read",
    });
  });

  it("ignores what it can't read", () => {
    localStorage.setItem("fctl:draft:usr_a", "{not json");
    expect(loadText("usr_a")).toBeUndefined();
    localStorage.setItem("fctl:draft:usr_a", JSON.stringify({ prompt: 3 }));
    expect(loadText("usr_a")).toBeUndefined();
    localStorage.setItem(
      "fctl:draft:usr_a",
      JSON.stringify({
        prompt: "Hi",
        project: 7,
        selection: [1, -2],
        model: 4,
        effort: null,
        github: "admin",
      }),
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

  it("keeps each project's apart from the home page's and the others'", async () => {
    const home = draftId("usr_a");
    const web = draftId("usr_a", "prj_web");
    const api = draftId("usr_a", "prj_api");
    expect(home).toBe("usr_a");
    saveText(home, { prompt: "On the home page", project: "web" });
    saveText(web, { prompt: "In web", model: "haiku" });
    saveText(api, { prompt: "In api" });
    saveText(draftId("usr_b", "prj_web"), { prompt: "Someone else's, in web" });
    setPageFiles(home, [shot()]);
    setPageFiles(web, [shot(), shot()]);
    expect(loadText(home)?.prompt).toBe("On the home page");
    expect(loadText(web)).toEqual({ prompt: "In web", model: "haiku" });
    expect(pageFiles(web)).toHaveLength(2);
    // A session started in web forgets web's alone.
    await forgetDraft(web);
    expect(loadText(web)).toBeUndefined();
    expect(pageFiles(web)).toBeUndefined();
    expect(loadText(home)?.prompt).toBe("On the home page");
    expect(pageFiles(home)).toHaveLength(1);
    expect(loadText(api)?.prompt).toBe("In api");
    // And one started on the home page, the home page's alone.
    await forgetDraft(home);
    expect(loadText(api)?.prompt).toBe("In api");
    // Signing out forgets them all.
    await forgetDraft();
    expect(loadText(api)).toBeUndefined();
    expect(loadText(draftId("usr_b", "prj_web"))).toBeUndefined();
  });

  it("keeps files only for the page where the browser stores none", async () => {
    // No IndexedDB here, as in some private windows.
    expect(await saveFile("usr_a", shot())).toBe(false);
    expect(await loadFiles("usr_a")).toEqual([]);
  });
});
