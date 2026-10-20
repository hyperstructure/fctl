import { describe, expect, it } from "vitest";
import { attachmentUrl, filesFromPaste, pastedName, playable, previewable, showsInline } from "./attachments";
import { bytes } from "./format";

function clipboard(files: File[], data: Record<string, string>): DataTransfer {
  const types = [...Object.keys(data), ...(files.length ? ["Files"] : [])];
  return { files, types, getData: (t: string) => data[t] ?? "" } as unknown as DataTransfer;
}

describe("attachments", () => {
  const png = new File(["x"], "image.png", { type: "image/png" });

  it("attaches pasted screenshots and copied files", () => {
    expect(filesFromPaste(clipboard([png], {}))).toEqual([png]);
    // Finder puts the file name next to the file.
    expect(filesFromPaste(clipboard([png], { "text/plain": "shot.png" }))).toEqual([png]);
    // "Copy image" in a browser: the picture and an <img> tag, no text.
    expect(filesFromPaste(clipboard([png], { "text/html": "<img src=x>" }))).toEqual([png]);
  });

  it("pastes rich text as text, not as its picture", () => {
    const word = clipboard([png], { "text/plain": "Quarterly plan", "text/html": "<p>Quarterly plan</p>" });
    expect(filesFromPaste(word)).toEqual([]);
    expect(filesFromPaste(clipboard([], { "text/plain": "hi" }))).toEqual([]);
    expect(filesFromPaste(null)).toEqual([]);
  });

  it("names pasted images", () => {
    expect(pastedName(png, 1)).toBe("pasted-image.png");
    expect(pastedName(new File(["x"], "image.jpeg", { type: "image/jpeg" }), 2)).toBe("pasted-image-2.jpg");
    expect(pastedName(new File(["x"], "", { type: "image/svg+xml" }), 3)).toBe("pasted-image-3.svg");
    expect(pastedName(new File(["x"], "Screenshot 2026-10-18.png", { type: "image/png" }), 1)).toBe(
      "Screenshot 2026-10-18.png",
    );
  });

  it("knows which images show in place", () => {
    expect(showsInline("image/webp")).toBe(true);
    expect(showsInline("image/svg+xml")).toBe(false);
    expect(previewable("image/svg+xml")).toBe(true);
    expect(previewable("image/heic")).toBe(false);
    expect(previewable("application/pdf")).toBe(false);
  });

  it("knows which videos play in place", () => {
    expect(playable("video/mp4")).toBe(true);
    expect(playable("video/webm")).toBe(true);
    expect(playable("video/quicktime")).toBe(true);
    expect(playable("video/x-msvideo")).toBe(false);
    expect(playable("audio/mpeg")).toBe(false);
    expect(playable("image/gif")).toBe(false);
  });

  it("builds file URLs and sizes", () => {
    expect(attachmentUrl("ses_1", "att_2")).toBe("/api/v1/sessions/ses_1/attachments/att_2");
    expect(bytes(912)).toBe("912 B");
    expect(bytes(245_300)).toBe("245 KB");
    expect(bytes(1_234_567)).toBe("1.2 MB");
    expect(bytes(20_000_000)).toBe("20 MB");
  });
});
