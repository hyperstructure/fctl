import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Gallery } from "@/components/ui/gallery";
import type * as G from "@/gen";
import type { DraftFile } from "@/lib/draft";
import { type Draft, DraftTray, MessageFiles, useAttachments } from "./attachments";

const { uploadAttachment, stored, onPage } = vi.hoisted(() => ({
  uploadAttachment: vi.fn(),
  stored: new Map<string, DraftFile[]>(),
  onPage: new Map<string, DraftFile[]>(),
}));
vi.mock("@/lib/api", () => ({ api: { uploadAttachment } }));
vi.mock("@/components/ui", async () => ({
  ...(await vi.importActual<typeof import("@/components/ui/lightbox")>("@/components/ui/lightbox")),
  ...(await vi.importActual<typeof import("@/components/ui/gallery")>("@/components/ui/gallery")),
  IconButton: () => null,
  toast: vi.fn(),
}));
// The browser's storage, in memory.
vi.mock("@/lib/draft", () => ({
  loadFiles: async (user: string) => stored.get(user) ?? [],
  saveFile: vi.fn(async (user: string, f: DraftFile) => {
    stored.set(user, [...(stored.get(user) ?? []), { key: f.key, name: f.name, file: f.file }]);
    return true;
  }),
  deleteFile: vi.fn(async (user: string, key: string) => {
    stored.set(
      user,
      (stored.get(user) ?? []).filter((f) => f.key !== key),
    );
  }),
  pageFiles: (user: string) => onPage.get(user),
  setPageFiles: (user: string, files: DraftFile[]) => onPage.set(user, files),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** Uploads that finish when the test says so. */
let pending: { resolve: (id: string) => void; abort: ReturnType<typeof vi.fn> }[] = [];
beforeEach(() => {
  pending = [];
  uploadAttachment.mockImplementation((_scope: unknown, file: File, name: string) => {
    let resolve: (id: string) => void = () => {};
    let reject: (e: Error) => void = () => {};
    const promise = new Promise((ok, fail) => {
      resolve = (id) => ok({ id, name, media_type: file.type, size: file.size });
      reject = fail;
    });
    const abort = vi.fn(() => reject(new Error("aborted")));
    pending.push({ resolve, abort });
    return { promise, abort };
  });
});

const unmounts: (() => void)[] = [];
afterEach(() => {
  for (const u of unmounts.splice(0)) u();
  stored.clear();
  onPage.clear();
  vi.clearAllMocks();
});

type Files = ReturnType<typeof useAttachments>;

function mount(keep?: string) {
  let current: Files | undefined;
  function Probe() {
    current = useAttachments({ project: "web" }, { keep });
    return null;
  }
  const root = createRoot(document.createElement("div"));
  act(() => root.render(<Probe />));
  let mounted = true;
  const unmount = () => {
    if (mounted) act(() => root.unmount());
    mounted = false;
  };
  unmounts.push(unmount);
  return { files: () => current as Files, unmount };
}

const settle = () => act(() => new Promise((r) => setTimeout(r, 0)));
const png = (name: string) => new File(["x"], name, { type: "image/png" });

describe("kept attachments", () => {
  it("come back ready when the page does, without uploading again", async () => {
    const first = mount("usr_a");
    await settle();
    act(() => first.files().add([png("shot.png")]));
    expect(uploadAttachment).toHaveBeenCalledTimes(1);
    expect(stored.get("usr_a")?.map((f) => f.name)).toEqual(["shot.png"]);
    await act(async () => pending[0]?.resolve("att_1"));
    expect(first.files().drafts[0]?.status).toBe("ready");
    first.unmount();
    expect(pending[0]?.abort).not.toHaveBeenCalled();

    const again = mount("usr_a");
    await settle();
    const [d] = again.files().drafts;
    expect(d).toMatchObject({ name: "shot.png", status: "ready", attachment: { id: "att_1" } });
    expect(uploadAttachment).toHaveBeenCalledTimes(1);
    const taken = await act(() => again.files().take());
    expect(taken.files.attachments.map((a) => a.id)).toEqual(["att_1"]);
  });

  it("upload again what was stored before the page opened", async () => {
    stored.set("usr_a", [{ key: "k1", name: "pasted-image.png", file: png("image.png") }]);
    const m = mount("usr_a");
    await settle();
    expect(m.files().drafts).toMatchObject([{ key: "k1", name: "pasted-image.png", status: "uploading" }]);
    expect(uploadAttachment).toHaveBeenCalledWith(
      { project: "web" },
      expect.any(File),
      "pasted-image.png",
      expect.any(Function),
    );
  });

  it("start their uploads again when the page left before they finished", async () => {
    const first = mount("usr_a");
    await settle();
    act(() => first.files().add([png("big.png")]));
    first.unmount();
    expect(pending[0]?.abort).toHaveBeenCalled();
    const again = mount("usr_a");
    await settle();
    expect(again.files().drafts[0]?.status).toBe("uploading");
    expect(uploadAttachment).toHaveBeenCalledTimes(2);
  });

  it("are forgotten when removed", async () => {
    const m = mount("usr_a");
    await settle();
    act(() => m.files().add([png("a.png"), png("b.png")]));
    const key = m.files().drafts[0]?.key ?? "";
    act(() => m.files().remove(key));
    await settle();
    expect(stored.get("usr_a")?.map((f) => f.name)).toEqual(["b.png"]);
    expect(onPage.get("usr_a")?.map((f) => f.name)).toEqual(["b.png"]);
  });

  it("stay in the composer as a session starts with them, and can be sent again if the start fails", async () => {
    const m = mount("usr_a");
    await settle();
    act(() => m.files().add([png("shot.png")]));
    await act(async () => pending[0]?.resolve("att_1"));
    const taken = await act(() => m.files().take({ stay: true }));
    expect(taken.files.attachments.map((a) => a.id)).toEqual(["att_1"]);
    expect(m.files().drafts).toMatchObject([{ name: "shot.png", status: "ready" }]);
    act(() => taken.restore());
    const again = await act(() => m.files().take({ stay: true }));
    expect(again.files.attachments.map((a) => a.id)).toEqual(["att_1"]);
    act(() => again.restore());
    // Kept for when the page comes back, uploads and all.
    m.unmount();
    const back = mount("usr_a");
    await settle();
    expect(back.files().drafts).toMatchObject([{ name: "shot.png", status: "ready" }]);
    expect(uploadAttachment).toHaveBeenCalledTimes(1);
  });

  it("stay in the composer when a session starts before they finish uploading", async () => {
    const m = mount("usr_a");
    await settle();
    act(() => m.files().add([png("shot.png")]));
    let taking: ReturnType<Files["take"]> = new Promise(() => {});
    act(() => {
      taking = m.files().take({ stay: true });
    });
    await act(async () => pending[0]?.resolve("att_1"));
    const taken = await act(() => taking);
    await settle();
    expect(taken.files.attachments.map((a) => a.id)).toEqual(["att_1"]);
    expect(m.files().drafts).toMatchObject([{ name: "shot.png", status: "ready" }]);
    expect(uploadAttachment).toHaveBeenCalledTimes(1);
  });

  it("let go of their uploads and previews once a session starts with them", async () => {
    const m = mount("usr_a");
    await settle();
    act(() => m.files().add([png("shot.png")]));
    await act(async () => pending[0]?.resolve("att_1"));
    const { preview } = m.files().drafts[0] ?? {};
    expect(preview).toBeTruthy();
    await act(() => m.files().take({ stay: true }));
    const revoke = vi.spyOn(URL, "revokeObjectURL");
    m.unmount();
    expect(revoke).toHaveBeenCalledWith(preview);
    revoke.mockRestore();
    // The session has the file: were the page to have it again, it would upload it again.
    const back = mount("usr_a");
    await settle();
    expect(back.files().drafts).toMatchObject([{ name: "shot.png", status: "uploading" }]);
    expect(uploadAttachment).toHaveBeenCalledTimes(2);
  });
});

describe("attachments of a message", () => {
  it("aren't kept", async () => {
    const m = mount();
    await settle();
    act(() => m.files().add([png("shot.png")]));
    expect(stored.size).toBe(0);
    m.unmount();
    expect(pending[0]?.abort).toHaveBeenCalled();
    expect(onPage.size).toBe(0);
  });

  it("leave the composer once sent, and upload once, even when sent while uploading", async () => {
    const m = mount();
    await settle();
    act(() => m.files().add([png("shot.png")]));
    let taking: ReturnType<Files["take"]> = new Promise(() => {});
    act(() => {
      taking = m.files().take();
    });
    await act(async () => pending[0]?.resolve("att_1"));
    const taken = await act(() => taking);
    await settle();
    expect(taken.files.attachments.map((a) => a.id)).toEqual(["att_1"]);
    expect(m.files().drafts).toEqual([]);
    expect(uploadAttachment).toHaveBeenCalledTimes(1);
    // Back as they are now if sending fails.
    act(() => taken.restore());
    expect(m.files().drafts).toMatchObject([
      { name: "shot.png", status: "ready", progress: 1, attachment: { id: "att_1" } },
    ]);
    await settle();
    expect(uploadAttachment).toHaveBeenCalledTimes(1);
  });
});

describe("files in a composer", () => {
  const draft: Draft = {
    key: "k1",
    file: png("shot.png"),
    name: "shot.png",
    preview: "blob:shot",
    status: "ready",
    progress: 1,
  };

  it("can't be removed while a start form starts with them", () => {
    const container = document.createElement("div");
    const root = createRoot(container);
    const tray = (readOnly: boolean) => (
      <DraftTray drafts={[draft]} onRemove={() => {}} onRetry={() => {}} readOnly={readOnly} />
    );
    act(() => root.render(tray(false)));
    unmounts.push(() => act(() => root.unmount()));
    expect(container.querySelector('button[aria-label="Remove shot.png"]')).not.toBeNull();
    act(() => root.render(tray(true)));
    expect(container.querySelector('[aria-label="Attached files"] img')?.getAttribute("alt")).toBe(
      "shot.png",
    );
    expect(container.querySelector('button[aria-label="Remove shot.png"]')).toBeNull();
  });
});

describe("files in the transcript", () => {
  const file = (id: string, name: string, media_type: string): G.Attachment => ({
    id,
    name,
    media_type,
    size: 3_400_000,
  });
  const show = (node: ReactNode) => {
    const container = document.createElement("div");
    const root = createRoot(container);
    act(() => root.render(node));
    unmounts.push(() => act(() => root.unmount()));
    return container;
  };

  it("plays videos over the page and downloads other files", () => {
    const c = show(
      <MessageFiles
        sessionId="ses_1"
        files={[file("att_1", "bug.mov", "video/quicktime"), file("att_2", "notes.txt", "text/plain")]}
      />,
    );
    act(() => c.querySelector<HTMLButtonElement>('button[aria-label="Play bug.mov"]')?.click());
    expect(document.querySelector('dialog[aria-label="bug.mov"] video')?.getAttribute("src")).toBe(
      "/api/v1/sessions/ses_1/attachments/att_1",
    );
    expect(c.querySelector('a[download="notes.txt"]')?.getAttribute("href")).toBe(
      "/api/v1/sessions/ses_1/attachments/att_2",
    );
  });

  it("shows images over the page", () => {
    const c = show(<MessageFiles sessionId="ses_1" files={[file("att_1", "after.png", "image/png")]} />);
    act(() => c.querySelector<HTMLButtonElement>('button[aria-label="View after.png"]')?.click());
    expect(document.querySelector('dialog[aria-label="after.png"] img')?.getAttribute("src")).toBe(
      "/api/v1/sessions/ses_1/attachments/att_1",
    );
  });

  it("steps from one image to the next in the gallery around them, and plays videos on their own", () => {
    const c = show(
      <Gallery>
        <MessageFiles
          sessionId="ses_1"
          files={[
            file("att_1", "before.png", "image/png"),
            file("att_2", "bug.mov", "video/quicktime"),
            file("att_3", "after.png", "image/png"),
          ]}
        />
      </Gallery>,
    );
    act(() => c.querySelector<HTMLButtonElement>('button[aria-label="View before.png"]')?.click());
    const images = document.querySelector("dialog") as HTMLDialogElement;
    expect(images.querySelector("p[aria-live]")?.textContent).toBe("before.png: 1 of 2");
    act(() => {
      images.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(images.getAttribute("aria-label")).toBe("after.png");
    act(() => images.close());
    // Its arrow keys seek, and fingers on it are the player's.
    act(() => c.querySelector<HTMLButtonElement>('button[aria-label="Play bug.mov"]')?.click());
    const video = document.querySelector("dialog") as HTMLDialogElement;
    expect(video.getAttribute("aria-label")).toBe("bug.mov");
    expect(video.querySelector("p[aria-live]")).toBeNull();
  });

  it("plays a video once it is sent", () => {
    const c = show(
      <MessageFiles sessionId="ses_1" files={[file("att_1", "bug.mov", "video/quicktime")]} pending />,
    );
    expect(c.querySelector("button")).toBeNull();
    expect(c.textContent).toContain("bug.mov");
  });
});
