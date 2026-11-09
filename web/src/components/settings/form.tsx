// What a page of settings that saves with a button is made of (a project's settings pages): a draft
// of what it edits, which knows whether it changed; a question before someone leaves it with
// changes unsaved; and switches whose words name them and turn them too.

import { type ShouldBlockFn, useBlocker } from "@tanstack/react-router";
import { type ReactNode, type RefObject, useCallback, useId, useState } from "react";
import { Button, Dialog, Switch } from "@/components/ui";
import { cn } from "@/lib/cn";

/** A value as a page compares it: its objects' keys in one order, and empty text as nothing typed. */
function plain(v: unknown): unknown {
  if (v === "") return undefined;
  if (Array.isArray(v)) return v.map(plain);
  if (v && typeof v === "object")
    return Object.fromEntries(
      Object.entries(v)
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([k, x]) => [k, plain(x)]),
    );
  return v;
}

/** Whether two drafts say the same: empty text is the same as none, and keys' order doesn't count. */
export const sameDraft = (a: unknown, b: unknown) => JSON.stringify(plain(a)) === JSON.stringify(plain(b));

/**
 * A page's draft of what it edits: what is saved, until someone changes it here; `dirty` while it
 * says something else; and what is saved again once that changes, by a save here or someone's
 * elsewhere, so that a page never saves what it showed before.
 */
export function useDraft<T>(saved: T) {
  const [draft, setDraft] = useState(saved);
  const stored = JSON.stringify(plain(saved));
  const [was, setWas] = useState(stored);
  if (stored !== was) {
    setWas(stored);
    setDraft(saved);
  }
  return {
    draft,
    setDraft,
    /** Changes some of it. */
    set: (patch: Partial<T>) => setDraft((d) => ({ ...d, ...patch })),
    dirty: !sameDraft(draft, saved),
    reset: () => setDraft(saved),
  };
}

// Another address, or this one with another `?tab=`: a page of its own, which drops the draft.
const leaves: ShouldBlockFn = ({ current, next }) =>
  current.pathname !== next.pathname || JSON.stringify(current.search) !== JSON.stringify(next.search);

/**
 * Asks before someone leaves a page whose changes aren't saved: for another page, another tab or
 * anywhere else in the app, and, as browsers ask it, before the tab closes or reloads. Not once the
 * page says it leaves on purpose (`leaving`): for a sign-in elsewhere that brings the browser back
 * to what it left, as connecting Linear does.
 */
export function UnsavedGuard({
  dirty,
  page,
  leaving,
}: {
  dirty: boolean;
  page: string;
  leaving?: RefObject<boolean>;
}) {
  const unload = useCallback(() => !leaving?.current, [leaving]);
  const blocker = useBlocker({
    shouldBlockFn: leaves,
    enableBeforeUnload: unload,
    disabled: !dirty,
    withResolver: true,
  });
  return (
    <Dialog
      open={blocker.status === "blocked"}
      onClose={() => blocker.reset?.()}
      title="Discard your changes?"
      description={`Your changes to ${page} aren't saved. Leaving this page drops them.`}
    >
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={() => blocker.reset?.()}>
          Keep editing
        </Button>
        <Button type="button" variant="danger" onClick={() => blocker.proceed?.()}>
          Discard changes
        </Button>
      </div>
    </Dialog>
  );
}

/**
 * A switch with what it does: its name, and under it what it does on, or what off means. A press on
 * the words turns it too, and screen readers read the words with it. `more` follows the words,
 * outside them, for what holds links of its own.
 */
export function SwitchRow({
  label,
  hint,
  checked,
  onChange,
  disabled,
  more,
}: {
  label: string;
  hint: ReactNode;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  more?: ReactNode;
}) {
  const id = useId();
  return (
    <div className="flex items-start justify-between gap-6 py-3.5">
      <div className="min-w-0">
        <label htmlFor={id} className={cn("block", !disabled && "cursor-pointer")}>
          <span className="block text-[13.5px] leading-5 text-ink">{label}</span>
          <span id={`${id}-hint`} className="mt-0.5 block text-[12.5px] leading-relaxed text-ink-4">
            {hint}
          </span>
        </label>
        {more}
      </div>
      <span className="mt-0.5 flex">
        <Switch
          id={id}
          checked={checked}
          onChange={onChange}
          label={label}
          describedBy={`${id}-hint`}
          disabled={disabled}
        />
      </span>
    </div>
  );
}

/** Switches one after another, between hairlines, flush with the page. */
export function SwitchRows({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cn("divide-y divide-line border-y border-line", className)}>{children}</div>;
}
