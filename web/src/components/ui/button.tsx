import { type ButtonHTMLAttributes, forwardRef, type ReactNode } from "react";
import { cn } from "@/lib/cn";
import { Spinner } from "./feedback";
import { fieldHeight } from "./form";

type Variant = "primary" | "secondary" | "ghost" | "danger" | "quiet";
type Size = "sm" | "md" | "field" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  /** `field` is as tall as an Input or a Select, for a button beside one. */
  size?: Size;
  loading?: boolean;
  icon?: ReactNode;
  trailing?: ReactNode;
}

// Disabled, a button doesn't respond to hover, and looks it, except while it's loading
// (`data-loading`): then it keeps its look beside the spinner, as the action it started goes on.
// The primary gives up its fill rather than fade, as the composers' Start does: faded yellow is a
// muddy olive on the dark page, a color the palette doesn't have.
const variants: Record<Variant, string> = {
  primary:
    "bg-highlight text-highlight-ink not-disabled:hover:brightness-[1.06] active:brightness-95 shadow-[inset_0_0_0_1px_rgb(23_23_23/0.1),0_1px_2px_rgb(0_0_0/0.3)] disabled:not-data-loading:bg-raised disabled:not-data-loading:text-ink-4 disabled:not-data-loading:shadow-[inset_0_0_0_1px_var(--c-line-strong)]",
  secondary:
    "bg-raised text-ink border border-line-strong not-disabled:hover:border-ink-4 not-disabled:hover:bg-raised/80 disabled:not-data-loading:text-ink-4",
  ghost:
    "text-ink-2 not-disabled:hover:text-ink not-disabled:hover:bg-hover/70 disabled:not-data-loading:text-ink-4",
  danger:
    "bg-danger-soft text-danger border border-danger/25 not-disabled:hover:bg-danger/15 disabled:not-data-loading:opacity-50",
  quiet: "text-ink-3 not-disabled:hover:text-ink disabled:not-data-loading:text-ink-4",
};

const sizes: Record<Size, string> = {
  sm: "h-7 px-2.5 text-[12.5px] gap-1.5 rounded-sm",
  md: "h-8.5 px-3.5 text-[13.5px] gap-2 rounded-md",
  field: `${fieldHeight} px-3.5 text-[13.5px] gap-2 rounded-md`,
  lg: "h-11 px-5 text-[15px] gap-2.5 rounded-lg",
};

/** A Button's look, for a link that goes where a button would: on to another site, say. */
export function buttonClass({
  variant = "secondary",
  size = "md",
  className,
}: {
  variant?: Variant;
  size?: Size;
  className?: string;
} = {}) {
  return cn(
    "relative inline-flex select-none items-center justify-center font-medium whitespace-nowrap",
    "transition-[background-color,border-color,color,transform,box-shadow] duration-150 ease-out active:scale-[0.97]",
    variants[variant],
    sizes[size],
    className,
  );
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", loading, icon, trailing, className, children, disabled, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      disabled={disabled || loading}
      data-loading={loading || undefined}
      className={buttonClass({ variant, size, className: cn("data-loading:cursor-progress", className) })}
      {...rest}
    >
      {loading ? <Spinner className="size-3.5" /> : icon}
      {children}
      {trailing}
    </button>
  );
});

export function IconButton({
  label,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex size-8 items-center justify-center rounded-md text-ink-3 transition-colors duration-150 hover:bg-hover hover:text-ink active:scale-95 disabled:pointer-events-none disabled:opacity-60",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}
