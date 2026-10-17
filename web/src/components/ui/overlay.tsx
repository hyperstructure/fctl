import { X } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { cn } from "@/lib/cn";

/** A modal dialog on the native <dialog> element (focus trap and Escape for free). */
export function Dialog({
  open,
  onClose,
  title,
  description,
  children,
  wide,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);
  return createPortal(
    // biome-ignore lint/a11y/useKeyWithClickEvents: the click closes on the backdrop only; Escape is native to <dialog>.
    <dialog
      ref={ref}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === ref.current) onClose();
      }}
      className={cn(
        "m-auto w-[calc(100vw-2rem)] rounded-xl border border-line-strong bg-surface p-0 text-ink shadow-[0_30px_120px_rgb(0_0_0/0.55)]",
        "backdrop:bg-black/45 backdrop:backdrop-blur-[3px] open:animate-rise",
        wide ? "max-w-2xl" : "max-w-md",
      )}
    >
      {open && (
        <div className="p-6">
          <div className="mb-5 flex items-start justify-between gap-4">
            <div>
              <h2 className="font-serif text-[22px] leading-tight tracking-[-0.01em]">{title}</h2>
              {description && (
                <p className="mt-1.5 text-[13.5px] leading-relaxed text-ink-3">{description}</p>
              )}
            </div>
            <button
              type="button"
              aria-label="Close"
              onClick={onClose}
              className="-mt-1 -mr-1 rounded-md p-1.5 text-ink-4 hover:bg-raised hover:text-ink"
            >
              <X className="size-4" />
            </button>
          </div>
          {children}
        </div>
      )}
    </dialog>,
    document.body,
  );
}

/** A small anchored menu. */
export function Menu({
  trigger,
  children,
  align = "end",
}: {
  trigger: (open: () => void) => ReactNode;
  children: (close: () => void) => ReactNode;
  align?: "start" | "end";
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);
  return (
    <div ref={ref} className="relative">
      {trigger(() => setOpen((o) => !o))}
      {open && (
        <div
          role="menu"
          className={cn(
            "absolute z-40 mt-1.5 min-w-48 origin-top animate-rise rounded-lg border border-line-strong bg-raised/95 p-1 shadow-[0_18px_60px_rgb(0_0_0/0.4)] backdrop-blur-xl",
            align === "end" ? "right-0" : "left-0",
          )}
        >
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

export function MenuItem({
  children,
  onClick,
  danger,
  icon,
}: {
  children: ReactNode;
  onClick: () => void;
  danger?: boolean;
  icon?: ReactNode;
}) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors",
        danger ? "text-danger hover:bg-danger-soft" : "text-ink-2 hover:bg-line hover:text-ink",
      )}
    >
      {icon && <span className="text-ink-4 [&_svg]:size-3.5">{icon}</span>}
      {children}
    </button>
  );
}
