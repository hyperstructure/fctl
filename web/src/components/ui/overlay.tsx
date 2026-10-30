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
        "top-(--viewport-shift) -bottom-(--viewport-shift) m-auto w-[calc(100vw-2rem)] rounded-xl border border-line-strong bg-surface p-0 text-ink shadow-[0_30px_120px_rgb(0_0_0/0.55)]",
        "backdrop:bg-black/45 backdrop:backdrop-blur-[3px] open:animate-rise",
        wide ? "max-w-2xl" : "max-w-md",
      )}
    >
      {open && (
        <div className="p-6">
          <div className="mb-5 flex items-start justify-between gap-4">
            <div className="min-w-0 [overflow-wrap:anywhere]">
              <h2 className="font-display text-[22px] leading-tight tracking-[-0.01em]">{title}</h2>
              {description && (
                <p className="mt-1.5 text-[13.5px] leading-relaxed text-ink-3">{description}</p>
              )}
            </div>
            <button
              type="button"
              aria-label="Close"
              onClick={onClose}
              className="-mt-1 -mr-1 rounded-md p-1.5 text-ink-4 hover:bg-hover hover:text-ink"
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

/**
 * A small anchored menu. It opens under its trigger, or over it with
 * `side="top"` when the trigger is at the bottom of the window.
 */
export function Menu({
  trigger,
  children,
  align = "end",
  side = "bottom",
  className,
}: {
  /** `open` is for the trigger's `aria-expanded`. */
  trigger: (toggle: () => void, open: boolean) => ReactNode;
  children: (close: () => void) => ReactNode;
  /** The trigger's edge it lines up with; `stretch` makes it as wide as the trigger. */
  align?: "start" | "end" | "stretch";
  side?: "top" | "bottom";
  /** For the box around the trigger, which the menu is placed against. */
  className?: string;
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
    <div ref={ref} className={cn("relative", className)}>
      {trigger(() => setOpen((o) => !o), open)}
      {open && (
        <div
          role="menu"
          className={cn(
            "absolute z-40 min-w-48 animate-rise rounded-lg border border-line-strong bg-raised/95 p-1 shadow-[0_18px_60px_rgb(0_0_0/0.4)] backdrop-blur-xl",
            side === "top" ? "bottom-full mb-1.5 origin-bottom" : "top-full mt-1.5 origin-top",
            align === "start" ? "left-0" : align === "end" ? "right-0" : "inset-x-0",
          )}
        >
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

/** A line between groups of a Menu's items. */
export function MenuSeparator() {
  return <hr className="-mx-1 my-1 border-line" />;
}

export function MenuItem({
  children,
  onClick,
  danger,
  icon,
  link,
  className,
}: {
  children: ReactNode;
  onClick: () => void;
  danger?: boolean;
  icon?: ReactNode;
  /** Makes the item a link; `onClick` still runs (to close the menu). */
  link?: { href: string; target?: string; rel?: string };
  className?: string;
}) {
  const classes = cn(
    "flex w-full items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-[13px] transition-colors pointer-coarse:py-2.5",
    danger ? "text-danger hover:bg-danger-soft" : "text-ink-2 hover:bg-line hover:text-ink",
    className,
  );
  const content = (
    <>
      {icon && <span className="text-ink-4 [&_svg]:size-3.5">{icon}</span>}
      {children}
    </>
  );
  return link ? (
    <a
      role="menuitem"
      href={link.href}
      target={link.target}
      rel={link.rel}
      onClick={onClick}
      className={classes}
    >
      {content}
    </a>
  ) : (
    <button type="button" role="menuitem" onClick={onClick} className={classes}>
      {content}
    </button>
  );
}
