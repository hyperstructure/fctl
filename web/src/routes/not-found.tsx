import { Link, useLocation } from "@tanstack/react-router";
import { StreamScreen } from "@/components/stream-screen";
import { Button } from "@/components/ui";
import { redact } from "@/lib/redact";
import { suggest } from "@/lib/suggest";

export function NotFound() {
  const { pathname } = useLocation();
  let shown = pathname;
  try {
    shown = decodeURI(pathname);
  } catch {}
  // A token pasted into the address by mistake doesn't end up in a screenshot.
  shown = redact(shown);
  const meant = suggest(pathname);
  return (
    <StreamScreen code="404">
      <h1 className="font-display text-[clamp(30px,4.4vw,48px)] font-medium leading-[1.08] tracking-[-0.025em] text-ink">
        <span className="sr-only">Error 404: </span>Nothing is being built here.
      </h1>
      <p className="mt-4 text-[15.5px] leading-relaxed text-ink-2">
        There’s no page at <code className="error-path">{shown}</code>. The link may be old, or the address
        mistyped.
      </p>
      {meant && (
        <p className="mt-2 text-[15.5px] leading-relaxed text-ink-2">
          Did you mean{" "}
          <a href={meant} className="error-path underline decoration-pink underline-offset-3">
            {meant}
          </a>
          ?
        </p>
      )}
      <div className="mt-8 flex justify-center">
        <Link to="/">
          <Button variant="primary" size="lg">
            Back home
          </Button>
        </Link>
      </div>
    </StreamScreen>
  );
}
