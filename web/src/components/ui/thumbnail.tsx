import { useState } from "react";

/** An image's size in pixels. */
interface Size {
  width: number;
  height: number;
}

/**
 * How big a thumbnail shows a `width` × `height` image in a row `row` px tall: as tall as the row and as wide as the
 * image's proportions make it, up to `widest`. A wider image shows whole at that width, shorter, and a taller one
 * narrower, but neither side gets under a third of the row: past that, the thumbnail cuts the image about its middle.
 */
export function thumbnailSize({ width, height }: Size, row: number, widest: number): Size {
  const ratio = width / height;
  const least = row / 3;
  const w = Math.min(row * ratio, widest);
  return {
    width: Math.round(Math.max(w, least)),
    height: Math.round(Math.min(Math.max(w / ratio, least), row)),
  };
}

/**
 * An image, small, in a frame of its caller's that takes its size: a row of them lines up at `row` px tall, and a
 * wide one fits `widest` (`thumbnailSize`). One whose size is known takes its room at once and loads as the reader
 * nears it; any other is as tall as the row until it loads.
 */
export function Thumbnail({
  src,
  alt,
  width,
  height,
  row,
  widest,
  onError,
}: {
  src?: string;
  alt: string;
  width?: number;
  height?: number;
  row: number;
  widest: number;
  onError?: () => void;
}) {
  const [loaded, setLoaded] = useState<Size>();
  const known = width && height ? { width, height } : loaded;
  return (
    <img
      src={src}
      alt={alt}
      width={width}
      height={height}
      loading="lazy"
      decoding="async"
      onLoad={(e) => {
        const { naturalWidth, naturalHeight } = e.currentTarget;
        if (naturalWidth && naturalHeight) setLoaded({ width: naturalWidth, height: naturalHeight });
      }}
      onError={onError}
      style={known ? thumbnailSize(known, row, widest) : { height: row }}
      // On a screen narrower than the thumbnail, it narrows with its frame and cuts the image instead.
      className="max-w-full object-cover"
    />
  );
}
