import type { Locale } from "@/i18n/config";

export type Currency = "SEK";

const currencies: Record<Locale, Currency> = { sv: "SEK", en: "SEK" };

/** Prices are integers in öre: 1995 is 19,95 kr. */
export function formatPrice(amount: number, locale: Locale): string {
  return new Intl.NumberFormat(locale, { style: "currency", currency: currencies[locale] }).format(
    amount / 100,
  );
}
