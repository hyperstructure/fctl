// Images agents read: those in the results of their tools (Claude Code's
// Read of a screenshot, a browser tool's), which the session keeps for the
// people who can see it for a week (fctl-domain `kept_images`), and the
// transcript shows under the calls that read them.

import type * as G from "@/gen";
import type { Json, ToolResultBlock } from "./claude-types";

/** How many days the session keeps them (`kept_images::TTL_MS`). */
export const KEPT_DAYS = 7;

/** An image in a tool's result. */
export interface ToolImage {
  /** `image/png` and the like, as Claude Code said. */
  mediaType?: string;
  /** Bytes, when it is known. */
  size?: number;
  /** The session's copy, which may have expired since. */
  kept?: G.KeptImage;
}

/** Whether the control plane said where it keeps an image: anything else is left out. */
function isKept(k: unknown): k is G.KeptImage {
  const o = k as Partial<G.KeptImage> | null | undefined;
  return (
    typeof o?.id === "string" &&
    typeof o.media_type === "string" &&
    typeof o.size === "number" &&
    typeof o.width === "number" &&
    typeof o.height === "number" &&
    typeof o.expires_at === "number"
  );
}

/** The images in a tool's result, in order. */
export function resultImages(content: ToolResultBlock["content"]): ToolImage[] {
  if (!Array.isArray(content)) return [];
  return content.flatMap((c): ToolImage[] => {
    if (c.type !== "image") return [];
    const s = c.source;
    const kept = isKept(s?.kept) ? s.kept : undefined;
    // What base64 took of it, as bytes.
    const omitted = typeof s?.omitted_bytes === "number" ? Math.floor((s.omitted_bytes * 3) / 4) : undefined;
    return [{ mediaType: kept?.media_type ?? s?.media_type, size: kept?.size ?? omitted, kept }];
  });
}

/** The session's copy of an image, while it has one. */
export const keptNow = (image: ToolImage, now = Date.now()): G.KeptImage | undefined =>
  image.kept && image.kept.expires_at > now ? image.kept : undefined;

export function keptImageUrl(session: string, id: string): string {
  return `/api/v1/sessions/${encodeURIComponent(session)}/images/${encodeURIComponent(id)}`;
}

const FORMATS: Record<string, string> = {
  "image/png": "PNG",
  "image/jpeg": "JPEG",
  "image/gif": "GIF",
  "image/webp": "WebP",
};

/** What an image's type is called: "PNG", "WebP". */
export const formatOf = (mediaType?: string): string | undefined =>
  mediaType ? (FORMATS[mediaType] ?? mediaType.replace(/^image\//, "").toUpperCase()) : undefined;

/**
 * What the `n`th image a call read is called: the name of the file it read, or else `image.png` (`image-2.png`, …),
 * after its type.
 */
export function imageName(input: Record<string, Json>, mediaType = "image/png", n = 0): string {
  const path = typeof input.file_path === "string" ? input.file_path : "";
  const file = path.split("/").pop();
  if (file) return n ? `${file} (${n + 1})` : file;
  const ext = mediaType === "image/jpeg" ? "jpg" : (mediaType.split("/")[1] ?? "png");
  return `image${n ? `-${n + 1}` : ""}.${ext}`;
}
