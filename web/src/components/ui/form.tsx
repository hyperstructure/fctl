import {
  forwardRef,
  type InputHTMLAttributes,
  type ReactNode,
  type TextareaHTMLAttributes,
  useId,
  useLayoutEffect,
  useRef,
} from "react";
import { cn } from "@/lib/cn";

const field =
  "w-full rounded-md border border-line-strong bg-sunken/60 px-3 text-[14px] text-ink placeholder:text-ink-4 " +
  "transition-[border-color,box-shadow,background-color] duration-150 outline-none " +
  "hover:border-ink-4/60 focus:border-accent/70 focus:bg-sunken focus:shadow-[0_0_0_3px_var(--c-accent-soft)] " +
  "aria-invalid:border-danger/60 aria-invalid:focus:shadow-[0_0_0_3px_var(--c-danger-soft)] disabled:opacity-60";

export const Input = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(function Input(
  { className, ...rest },
  ref,
) {
  return <input ref={ref} className={cn(field, "h-9.5", className)} {...rest} />;
});

/** A textarea that grows with its content up to `maxRows`. */
export const Textarea = forwardRef<
  HTMLTextAreaElement,
  TextareaHTMLAttributes<HTMLTextAreaElement> & { maxRows?: number; minRows?: number; bare?: boolean }
>(function Textarea({ className, maxRows = 14, minRows = 2, bare, value, ...rest }, outer) {
  const ref = useRef<HTMLTextAreaElement | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: re-measure whenever the value changes
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    const line = Number.parseFloat(getComputedStyle(el).lineHeight) || 22;
    const pad = el.offsetHeight - el.clientHeight + 16;
    el.style.height = `${Math.min(Math.max(el.scrollHeight, line * minRows + pad), line * maxRows + pad)}px`;
  }, [value, maxRows, minRows]);
  return (
    <textarea
      ref={(el) => {
        ref.current = el;
        if (typeof outer === "function") outer(el);
        else if (outer) outer.current = el;
      }}
      value={value}
      className={cn(
        bare
          ? "w-full resize-none bg-transparent outline-none placeholder:text-ink-4"
          : cn(field, "resize-none py-2 leading-relaxed"),
        className,
      )}
      {...rest}
    />
  );
});

export function Field({
  label,
  hint,
  error,
  children,
  className,
  action,
}: {
  label: ReactNode;
  hint?: ReactNode;
  error?: string;
  children: (id: string) => ReactNode;
  className?: string;
  action?: ReactNode;
}) {
  const id = useId();
  return (
    <div className={cn("flex flex-col gap-1.5", className)}>
      <div className="flex items-baseline justify-between">
        <label htmlFor={id} className="text-[12.5px] font-medium text-ink-2">
          {label}
        </label>
        {action}
      </div>
      {children(id)}
      {error ? (
        <p className="animate-rise text-[12.5px] text-danger" role="alert">
          {error}
        </p>
      ) : hint ? (
        <p className="text-[12.5px] text-ink-3">{hint}</p>
      ) : null}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative h-5 w-9 shrink-0 rounded-full border transition-colors duration-200 disabled:opacity-50",
        checked ? "border-highlight bg-highlight" : "border-line-strong bg-raised",
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 left-0.5 size-3.5 rounded-full shadow-sm transition-[translate,background-color] duration-250 ease-[var(--ease-spring)]",
          checked ? "translate-x-4 bg-highlight-ink" : "bg-ink-3",
        )}
      />
    </button>
  );
}
