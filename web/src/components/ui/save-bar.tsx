// The bar that rises at the bottom of the screen once something on a page of settings changed: what
// saving does, or what is missing, and Cancel and Save. A project's settings pages and its tickets
// save with it; with nothing changed there is nothing to save, and it isn't there.

import { Button } from "./button";

export function SaveBar({
  pending,
  disabled,
  hint,
  onSave,
  onReset,
}: {
  pending: boolean;
  disabled?: boolean;
  hint: string;
  /** Saves; without it, Save submits the form the bar is in. */
  onSave?: () => void;
  onReset: () => void;
}) {
  return (
    <div className="sticky bottom-[calc(1rem-var(--viewport-shift))] z-10 mt-8 flex animate-rise flex-wrap items-center justify-between gap-3 rounded-xl border border-line-strong bg-surface/95 px-4 py-3 shadow-[0_18px_50px_-24px_rgb(0_0_0/0.45)] backdrop-blur max-sm:bottom-[calc(0.5rem-var(--viewport-shift))]">
      <span aria-live="polite" className="text-[12.5px] text-ink-3">
        {hint}
      </span>
      <div className="flex items-center gap-2">
        <Button type="button" variant="ghost" onClick={onReset} disabled={pending}>
          Cancel
        </Button>
        <Button
          type={onSave ? "button" : "submit"}
          variant="primary"
          loading={pending}
          disabled={disabled}
          onClick={onSave}
        >
          Save
        </Button>
      </div>
    </div>
  );
}
