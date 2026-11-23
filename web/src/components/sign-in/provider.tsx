// A sign-in provider's mark and button ("Continue with Google"), as the sign-in page, invitations and Settings show
// them. Each provider's button keeps its own mark and colors; one this app has no mark for gets a key.

import { ArrowRight, KeyRound } from "lucide-react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { GOOGLE, startHref } from "@/lib/sign-in";

function GoogleMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} aria-hidden="true">
      <path
        fill="#4285F4"
        d="M23.5 12.3c0-.8-.1-1.6-.2-2.3H12v4.4h6.5a5.6 5.6 0 0 1-2.4 3.6v3h3.9c2.3-2.1 3.5-5.2 3.5-8.7z"
      />
      <path
        fill="#34A853"
        d="M12 24c3.2 0 6-1.1 8-2.9l-3.9-3c-1.1.7-2.5 1.2-4.1 1.2-3.1 0-5.8-2.1-6.7-5H1.3v3.1A12 12 0 0 0 12 24z"
      />
      <path fill="#FBBC05" d="M5.3 14.3a7.2 7.2 0 0 1 0-4.6V6.6H1.3a12 12 0 0 0 0 10.8l4-3.1z" />
      <path
        fill="#EA4335"
        d="M12 4.8c1.8 0 3.3.6 4.6 1.8l3.4-3.4A12 12 0 0 0 1.3 6.6l4 3.1c.9-2.9 3.6-4.9 6.7-4.9z"
      />
    </svg>
  );
}

/** A provider's mark, by its id. */
export function ProviderMark({ id, className }: { id: string; className?: string }) {
  if (id === GOOGLE) return <GoogleMark className={className} />;
  return <KeyRound className={className} aria-hidden="true" />;
}

/** An address's name and its `@domain`, for cutting it short between the two. */
function splitAddress(email: string): [string, string] {
  const at = email.lastIndexOf("@");
  return at > 0 ? [email.slice(0, at), email.slice(at)] : [email, ""];
}

const frame =
  "flex w-full items-center rounded-lg border border-line-strong bg-raised/80 text-ink transition-colors hover:border-ink-4";

/** The two lines of a provider's button with an address: its label, then the address, cut short as it must. */
function WithAddress({ provider, email }: { provider: G.SignInProvider; email: string }) {
  const [name, domain] = splitAddress(email);
  return (
    <>
      <ProviderMark id={provider.id} className="size-5 shrink-0" />
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[14px] leading-tight font-medium">Continue with {provider.name}</span>
        {/* The name gives way first, and the domain keeps at most two thirds of the line. */}
        <span className="flex min-w-0 text-[12.5px] leading-tight text-ink-3">
          <span className="min-w-0 truncate">{name}</span>
          <span className="max-w-2/3 shrink-0 truncate">{domain}</span>
        </span>
      </span>
      <ArrowRight className="size-4 shrink-0 text-ink-3 transition-transform duration-200 group-hover:translate-x-0.5 group-hover:text-ink motion-reduce:transition-none" />
    </>
  );
}

const withAddress = "group h-14 gap-3 pr-3.5 pl-4 text-left";

/**
 * Signs in with `provider`, then goes on to `next`. With `email` (an invitation's address), the address goes on a
 * line of its own under the label, and a long one is cut short, its name sooner than its domain: the button stays
 * one height however long it is. The provider offers that address's account first.
 */
export function ProviderButton({
  provider,
  next,
  email,
}: {
  provider: G.SignInProvider;
  next?: string;
  email?: string;
}) {
  const href = startHref(provider, { next, hint: email });
  if (!email) {
    return (
      <a href={href} className={cn(frame, "h-10 justify-center gap-2.5 text-[14px] font-medium")}>
        <ProviderMark id={provider.id} className="size-4 shrink-0" />
        Continue with {provider.name}
      </a>
    );
  }
  return (
    <a
      href={href}
      title={email}
      aria-label={`Continue with ${provider.name} as ${email}`}
      className={cn(frame, withAddress)}
    >
      <WithAddress provider={provider} email={email} />
    </a>
  );
}

/**
 * A form's submit button that is a provider's: the sign-in form becomes it once the address typed is at a domain
 * that signs in with `provider`. The form sends people on to it.
 */
export function ProviderSubmit({ provider, email }: { provider: G.SignInProvider; email: string }) {
  return (
    <button
      type="submit"
      title={email}
      aria-label={`Continue with ${provider.name} as ${email}`}
      className={cn(frame, withAddress, "animate-rise")}
    >
      <WithAddress provider={provider} email={email} />
    </button>
  );
}
