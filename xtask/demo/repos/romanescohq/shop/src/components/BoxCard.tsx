import { formatPrice } from "@/lib/money";
import type { Locale } from "@/i18n/config";

type Box = { id: string; name: string; price: number; image: string };

export function BoxCard({ box, locale }: { box: Box; locale: Locale }) {
  return (
    <article className="box-card">
      <img src={box.image} alt="" className="box-card__image" />
      <h3>{box.name}</h3>
      <p className="box-card__price">
        {formatPrice(box.price, locale)}
      </p>
    </article>
  );
}
