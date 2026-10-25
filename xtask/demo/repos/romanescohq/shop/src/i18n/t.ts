import en from "../../messages/en.json";
import sv from "../../messages/sv.json";
import type { Locale } from "./config";

const messages: Partial<Record<Locale, Record<string, string>>> = { sv, en };

/** The message `key` in `locale` (Swedish when it has none), `{name}`s filled in. */
export function t(locale: Locale, key: string, values: Record<string, string> = {}): string {
  const text = messages[locale]?.[key] ?? messages.sv?.[key] ?? key;
  return text.replace(/\{(\w+)\}/g, (_, name: string) => values[name] ?? `{${name}}`);
}
