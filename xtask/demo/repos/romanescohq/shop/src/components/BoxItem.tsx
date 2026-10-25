import type { Locale } from "@/i18n/config";
import { formatPrice } from "@/lib/money";

export type Item = { id: string; name: string; weight: string; price: number; farmId: string };

/** One vegetable, loaf or dozen eggs in a box: in the box builder and on the box's page. */
export function BoxItem({ item, locale }: { item: Item; locale: Locale }) {
  return (
    <li className="box-item">
      <span className="box-item__name">{item.name}</span>
      <span className="box-item__weight">{item.weight}</span>
      <span className="box-item__price">{formatPrice(item.price, locale)}</span>
    </li>
  );
}
