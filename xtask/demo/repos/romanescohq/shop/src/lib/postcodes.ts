import zones from "@/data/zones.json";

export type Zone = { id: string; depot: string; name: string };

/** "114 32", "11432" and " 114 32 " are one postcode; anything else is none. */
export function normalize(postcode: string): string | null {
  const digits = postcode.replace(/\s+/g, "");
  return /^\d{5}$/.test(digits) ? digits : null;
}

/**
 * The delivery zones a postcode is in: none outside them, and two for a
 * postcode on the border between two, where either zone's vans may come.
 */
export function zonesFor(postcode: string): Zone[] {
  const p = normalize(postcode);
  if (!p) return [];
  return zones
    .filter((z) => z.ranges.some(([from, to]) => p >= from && p <= to))
    .map(({ id, depot, name }) => ({ id, depot, name }));
}
