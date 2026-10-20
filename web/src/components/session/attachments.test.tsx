import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DraftFile } from "@/lib/draft";
import { useAttachments } from "./attachments";

const { uploadAttachment, stored, onPage } = vi.hoisted(() => ({
  uploadAttachment: vi.fn(),
  stored: new Map<string, DraftFile[]>(),
  onPage: new Map<string, DraftFile[]>(),
}));
vi.mock("@/lib/api", () => ({ api: { uploadAttachment } }));
vi.mock("@/components/ui", () => ({ IconButton: () => null, toast: vi.fn() }));
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

  it("go back in the composer when the start fails, uploads and all", async () => {
    const m = mount("usr_a");
    await settle();
    act(() => m.files().add([png("shot.png")]));
    await act(async () => pending[0]?.resolve("att_1"));
    const taken = await act(() => m.files().take());
    expect(m.files().drafts).toEqual([]);
    act(() => taken.restore());
    expect(m.files().drafts).toHaveLength(1);
    const again = await act(() => m.files().take());
    expect(again.files.attachments.map((a) => a.id)).toEqual(["att_1"]);
    expect(uploadAttachment).toHaveBeenCalledTimes(1);
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
});
