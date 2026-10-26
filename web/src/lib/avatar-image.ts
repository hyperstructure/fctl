// Making an uploaded picture an avatar, a person's or a workspace's: the
// centered square of it, 256 px a side, as PNG, which the control plane
// takes (at most 256 KB).

const SIDE = 256;

/** The largest centered square of an image of `w`×`h`: [sx, sy, side]. */
export function centerSquare(w: number, h: number): [number, number, number] {
  const side = Math.min(w, h);
  return [Math.round((w - side) / 2), Math.round((h - side) / 2), side];
}

/** The picture's centered square at 256 px, as a PNG (WebP when it comes out smaller). */
export async function avatarImage(file: Blob): Promise<Blob> {
  const bitmap = await createImageBitmap(file);
  try {
    const canvas = document.createElement("canvas");
    canvas.width = SIDE;
    canvas.height = SIDE;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("This browser can't resize images");
    ctx.imageSmoothingQuality = "high";
    const [sx, sy, side] = centerSquare(bitmap.width, bitmap.height);
    ctx.drawImage(bitmap, sx, sy, side, side, 0, 0, SIDE, SIDE);
    const encode = (type: string, quality?: number) =>
      new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, quality));
    const png = await encode("image/png");
    const webp = await encode("image/webp", 0.9);
    const best = [png, webp]
      .filter((b): b is Blob => !!b && (b.type === "image/png" || b.type === "image/webp"))
      .sort((a, b) => a.size - b.size)[0];
    if (!best) throw new Error("This browser can't make the image");
    return best;
  } finally {
    bitmap.close();
  }
}
