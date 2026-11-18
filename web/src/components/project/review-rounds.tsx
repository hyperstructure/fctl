// How many reviews in a row reach the agent that opened a pull request while nobody messages it, in the
// review agents' settings: then fctl pauses, holding the next findings back until a person steps in, so
// that the two agents don't go back and forth for good.

import { Select, type SelectOption } from "@/components/ui";
import { DEFAULT_ROUNDS, MAX_ROUNDS } from "@/lib/reviews";

const OPTIONS: SelectOption[] = Array.from({ length: MAX_ROUNDS }, (_, i) => {
  const n = i + 1;
  return {
    value: String(n),
    label: `${n} ${n === 1 ? "round" : "rounds"}`,
    description: n === DEFAULT_ROUNDS ? "fctl's default" : undefined,
  };
});

/** The choice, with what it does now; the form saves it. */
export function ReviewRounds({ value, onChange }: { value: number; onChange: (n: number) => void }) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0">
        <p className="text-[12.5px] font-medium text-ink-2">Review rounds before a pause</p>
        <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
          When one of fctl's agents opened the pull request, it gets the findings of{" "}
          {value === 1 ? "one review" : `${value} reviews in a row`} while nobody writes to it. Then fctl
          holds the next findings back until someone messages the agent or passes them on from its session.
        </p>
      </div>
      <Select
        size="sm"
        value={String(value)}
        options={OPTIONS}
        onChange={(v) => onChange(Number(v))}
        aria-label="Review rounds before a pause"
        className="shrink-0"
      />
    </div>
  );
}
