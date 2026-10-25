import { formatPrice } from "@/lib/money";
import type { Locale } from "@/i18n/config";

type Props = { order: { total: number }; locale: Locale };

export function Summary({ order, locale }: Props) {
  // Prices are öre/øre integers.
  const shown = formatPrice(order.total, locale);
  return <p className="summary-total">{shown}</p>;
}
