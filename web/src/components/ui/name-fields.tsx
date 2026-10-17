import { Field, Input } from "./form";

/** First names and last name, side by side (stacked on phones). */
export function NameFields({
  given,
  family,
  onGiven,
  onFamily,
  error,
  autoFocus,
}: {
  given: string;
  family: string;
  onGiven: (v: string) => void;
  onFamily: (v: string) => void;
  /** The error for a field (`given_name`, `family_name`), if any. */
  error?: (field: string) => string | undefined;
  autoFocus?: boolean;
}) {
  return (
    <div className="grid gap-4 sm:grid-cols-2">
      <Field label="First name" hint="All of them, if you go by more than one." error={error?.("given_name")}>
        {(id) => (
          <Input
            id={id}
            required
            autoComplete="given-name"
            value={given}
            onChange={(e) => onGiven(e.target.value)}
            autoFocus={autoFocus}
          />
        )}
      </Field>
      <Field label="Last name" error={error?.("family_name")}>
        {(id) => (
          <Input
            id={id}
            autoComplete="family-name"
            value={family}
            onChange={(e) => onFamily(e.target.value)}
          />
        )}
      </Field>
    </div>
  );
}

/** First names and last name of a user, for forms; older accounts may lack them. */
export function nameParts(user: { name: string; given_name?: string; family_name?: string }) {
  if (user.given_name) return { given: user.given_name, family: user.family_name ?? "" };
  const words = user.name.trim().split(/\s+/);
  return words.length > 1
    ? { given: words.slice(0, -1).join(" "), family: words[words.length - 1] ?? "" }
    : { given: user.name.trim(), family: "" };
}

/** What greetings call someone: all their first names. */
export function firstNames(user: { name: string; given_name?: string; family_name?: string }) {
  return user.given_name ?? nameParts(user).given;
}
