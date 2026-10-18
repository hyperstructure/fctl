import { Fragment, forwardRef, useEffect, useState } from "react";
import { cn } from "@/lib/cn";
import { CODE_LENGTH, digits } from "@/lib/two-factor";

export type CodeState = "idle" | "busy" | "error" | "ok";

/**
 * A six-digit code from an authenticator app. One input takes it, so that
 * pasting, the phone's one-time-code suggestions and password managers fill
 * it at once; it shows as boxes, in two groups of three.
 */
export const CodeInput = forwardRef<
  HTMLInputElement,
  {
    value: string;
    onChange: (value: string) => void;
    /** Called when the last digit comes in. */
    onComplete?: (value: string) => void;
    state?: CodeState;
    /** Changes with each wrong code, to shake the boxes again. */
    errors?: number;
    disabled?: boolean;
    autoFocus?: boolean;
    id?: string;
    label?: string;
    describedBy?: string;
  }
>(function CodeInput(
  { value, onChange, onComplete, state = "idle", errors = 0, disabled, autoFocus, id, label, describedBy },
  ref,
) {
  const [focused, setFocused] = useState(false);
  const [shaking, setShaking] = useState(false);
  useEffect(() => {
    if (errors > 0) setShaking(true);
  }, [errors]);
  const at = Math.min(value.length, CODE_LENGTH - 1);
  const typing = focused && !disabled && state !== "busy" && state !== "ok";
  return (
    <div
      className={cn("relative inline-flex items-center gap-1.5 sm:gap-2", shaking && "animate-shake")}
      onAnimationEnd={(e) => {
        if (e.animationName === "shake") setShaking(false);
      }}
    >
      {Array.from({ length: CODE_LENGTH }, (_, i) => {
        const digit = value[i];
        const current = typing && i === at;
        return (
          <Fragment key={i}>
            {i === CODE_LENGTH / 2 && <span className="w-1 sm:w-2" aria-hidden="true" />}
            <span
              aria-hidden="true"
              className={cn(
                "relative flex h-12 w-10 items-center justify-center rounded-md border font-mono text-[22px] font-medium text-ink sm:h-[52px] sm:w-11",
                "transition-[border-color,box-shadow,background-color] duration-150",
                state === "ok"
                  ? "border-accent/60 bg-accent-soft"
                  : state === "error"
                    ? "border-danger/60 bg-sunken/60"
                    : current
                      ? "border-accent/70 bg-sunken shadow-[0_0_0_3px_var(--c-accent-soft)]"
                      : "border-line-strong bg-sunken/60",
                (state === "busy" || disabled) && state !== "ok" && "opacity-55",
              )}
              style={state === "ok" ? { transitionDelay: `${i * 35}ms` } : undefined}
            >
              {digit ? (
                <span key={`${i}-${digit}`} className="animate-pop">
                  {digit}
                </span>
              ) : current ? (
                <span className="h-6 w-px animate-caret bg-ink" />
              ) : null}
            </span>
          </Fragment>
        );
      })}
      <input
        ref={ref}
        id={id}
        value={value}
        disabled={disabled}
        autoFocus={autoFocus}
        inputMode="numeric"
        autoComplete="one-time-code"
        pattern="[0-9]*"
        spellCheck={false}
        aria-label={label}
        aria-describedby={describedBy}
        aria-invalid={state === "error" || undefined}
        className="absolute inset-0 h-full w-full cursor-text rounded-md bg-transparent text-[16px] text-transparent caret-transparent outline-none selection:bg-transparent focus-visible:outline-none disabled:cursor-default"
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onKeyDown={(e) => {
          // The caret stays at the end: the boxes show where it is.
          if (["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) e.preventDefault();
        }}
        onChange={(e) => {
          const next = digits(e.target.value);
          onChange(next);
          if (next.length === CODE_LENGTH && next !== value) onComplete?.(next);
        }}
      />
    </div>
  );
});
