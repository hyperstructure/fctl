import { type CSSProperties, useMemo } from "react";
import { cn } from "@/lib/cn";
import { qrRings } from "@/lib/two-factor";

/**
 * A QR code, dark on white in either theme, as every scanner wants it, with
 * its quiet zone. It appears from the middle out.
 */
export function QrCode({ rows, label, className }: { rows: string[]; label: string; className?: string }) {
  const rings = useMemo(() => qrRings(rows), [rows]);
  const size = rows.length + 8;
  return (
    <svg
      viewBox={`0 0 ${size} ${size}`}
      role="img"
      aria-label={label}
      shapeRendering="crispEdges"
      className={cn(
        "block rounded-lg shadow-[0_8px_30px_rgb(0_0_0/0.18)] ring-1 ring-line-strong",
        className,
      )}
    >
      <rect width={size} height={size} rx={2} fill="#ffffff" />
      {rings.map((r) => (
        <path
          key={r.ring}
          d={r.d}
          fill="#171717"
          className="qr-ring"
          style={{ "--ring": r.ring } as CSSProperties}
        />
      ))}
    </svg>
  );
}
