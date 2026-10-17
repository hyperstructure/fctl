import { type ReactNode, useId } from "react";
import { Button } from "./button";

/** A bordered danger zone with one destructive action (its dialog goes in `children`). */
export function DangerZone({
  title,
  description,
  action,
  onAction,
  children,
}: {
  title: ReactNode;
  description: ReactNode;
  action: ReactNode;
  onAction: () => void;
  children?: ReactNode;
}) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="mt-6 rounded-xl border border-danger/30 p-6">
      <h2 id={id} className="text-[14px] font-semibold text-danger">
        Danger zone
      </h2>
      <div className="mt-4 flex flex-col gap-4 border-t border-danger/20 pt-4 sm:flex-row sm:items-center sm:justify-between">
        <div className="max-w-xl">
          <div className="text-[13.5px] font-medium text-ink">{title}</div>
          <p className="mt-1 text-[13px] leading-relaxed text-ink-3">{description}</p>
        </div>
        <Button variant="danger" className="self-start sm:self-auto" onClick={onAction}>
          {action}
        </Button>
      </div>
      {children}
    </section>
  );
}
