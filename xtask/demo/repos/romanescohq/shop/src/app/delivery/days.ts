import type { Locale } from "@/i18n/config";

const DEPOT = "sto-1";

export async function deliveryDays(postcode: string, _locale: Locale) {
  const res = await fetch(`/api/depots/${DEPOT}/slots?postcode=${postcode}`);
  return res.json();
}
