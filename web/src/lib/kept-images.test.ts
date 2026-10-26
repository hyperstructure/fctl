import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { formatOf, imageName, keptImageUrl, keptNow, resultImages } from "./kept-images";

const kept: G.KeptImage = {
  id: "img_01",
  media_type: "image/png",
  size: 125_879,
  width: 1526,
  height: 702,
  expires_at: 2_000,
};
const image = (source: Record<string, unknown>) => ({ type: "image", source });

describe("the images in a tool's result", () => {
  it("says where the session keeps each, and how large it is", () => {
    const content = [
      { type: "text", text: "Took a screenshot" },
      image({ type: "base64", media_type: "image/png", data: "", omitted_bytes: 167_840, kept }),
      // Stored before sessions kept them: what base64 took of it.
      image({ type: "base64", media_type: "image/jpeg", data: "", omitted_bytes: 4_000 }),
    ];
    expect(resultImages(content)).toEqual([
      { mediaType: "image/png", size: 125_879, kept },
      { mediaType: "image/jpeg", size: 3_000, kept: undefined },
    ]);
    expect(resultImages("12 lines")).toEqual([]);
    expect(resultImages(undefined)).toEqual([]);
  });

  it("leaves out a copy the control plane didn't describe", () => {
    const odd = { id: "img_01", media_type: "image/png" } as unknown as G.KeptImage;
    expect(resultImages([image({ type: "base64", media_type: "image/png", kept: odd })])).toEqual([
      { mediaType: "image/png", size: undefined, kept: undefined },
    ]);
  });

  it("are kept until they expire", () => {
    expect(keptNow({ kept }, 1_999)).toBe(kept);
    expect(keptNow({ kept }, 2_000)).toBeUndefined();
    expect(keptNow({ mediaType: "image/png" }, 0)).toBeUndefined();
  });

  it("come from the session that keeps them", () => {
    expect(keptImageUrl("ses_1", "img_01")).toBe("/api/v1/sessions/ses_1/images/img_01");
  });

  it("are called after the file the call read, or else after their type", () => {
    expect(imageName({ file_path: "/home/agent/screenshots/after.png" }, "image/png")).toBe("after.png");
    expect(imageName({ file_path: "/tmp/shot.png" }, "image/png", 1)).toBe("shot.png (2)");
    expect(imageName({ fullPage: true }, "image/jpeg")).toBe("image.jpg");
    expect(imageName({}, "image/webp", 2)).toBe("image-3.webp");
    expect(formatOf("image/webp")).toBe("WebP");
    expect(formatOf("image/png")).toBe("PNG");
    expect(formatOf(undefined)).toBeUndefined();
  });
});
