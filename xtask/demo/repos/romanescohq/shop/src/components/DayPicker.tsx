"use client";

import { useState } from "react";
import type { Slot } from "@/app/delivery/slots";
import type { Locale } from "@/i18n/config";
import { t } from "@/i18n/t";
import { useShortViewport } from "@/lib/viewport";

type Props = { slots: Slot[]; locale: Locale; onChange: (date: string) => void };

/** Two weeks of delivery days: a grid, or a list when the screen is short. */
export function DayPicker({ slots, locale, onChange }: Props) {
  const short = useShortViewport();
  const layout = short ? "list" : "grid";
  return (
    <div className="day-picker">
      <label htmlFor="delivery-day">{t(locale, "delivery.pick")}</label>
      <Days key={layout} layout={layout} slots={slots} onChange={onChange} />
    </div>
  );
}

type DaysProps = { layout: "list" | "grid"; slots: Slot[]; onChange: (date: string) => void };

function Days({ layout, slots, onChange }: DaysProps) {
  const [selected, setSelected] = useState<string | null>(null);
  const chosen = slots.find((s) => s.date === selected);
  return (
    <div className={`day-picker__days day-picker__days--${layout}`}>
      <input
        id="delivery-day"
        className="day-picker__value"
        value={chosen?.label ?? ""}
        readOnly
      />
      <div role="listbox" aria-labelledby="delivery-day">
        {slots.map((s) => (
          <button
            key={s.date}
            type="button"
            role="option"
            aria-selected={s.date === selected}
            disabled={s.full}
            className="day-picker__day"
            onClick={() => {
              setSelected(s.date);
              onChange(s.date);
            }}
          >
            {s.label}
          </button>
        ))}
      </div>
    </div>
  );
}
