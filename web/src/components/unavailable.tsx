// What a page shows in place of what it is about when that didn't load: a project, a workspace or a
// session that isn't there for the reader, one that couldn't be reached just now, and Admin for
// someone who isn't one. Calm rather than red, as nothing failed that the reader can fix by being
// warned, and with the way on: back to the Agent Floor, or to try again.

import { Link } from "@tanstack/react-router";
import { CloudOff, LockKeyhole, SearchX } from "lucide-react";
import type { ReactNode } from "react";
import { Button, Kbd } from "@/components/ui";
import { ApiFailure } from "@/lib/api";
import { shortcutText } from "@/lib/shortcuts";

/**
 * Whether a failed load means the thing isn't there for this reader, gone or never shown to them,
 * rather than that it couldn't be reached. The server answers both the same (one sentence for both,
 * so that the page doesn't say which); a 403 that asks for something, such as two-factor sign-in,
 * says what in its message instead.
 */
export function isMissing(error: unknown): boolean {
  if (!(error instanceof ApiFailure)) return false;
  return (
    error.status === 404 || (error.status === 403 && ["forbidden", "access_denied"].includes(error.code))
  );
}

type What = "project" | "workspace" | "session";

const MISSING: Record<What, { title: string; text: string }> = {
  project: {
    title: "No project here",
    text: "This project does not exist, or you cannot see it. If someone sent you its link, they can add you to it.",
  },
  workspace: {
    title: "No workspace here",
    text: "This workspace does not exist, or you aren't a member of it. Its admins can invite you.",
  },
  session: {
    title: "No session here",
    text: "This session does not exist, or you cannot see it. If someone sent you its link, they can add you to its project.",
  },
};

/** A project, a workspace or a session that didn't load: why, as far as the page can tell, and the way on. */
export function Unavailable({
  what,
  error,
  onRetry,
  retrying,
}: {
  what: What;
  error: unknown;
  onRetry: () => void;
  retrying?: boolean;
}) {
  if (isMissing(error)) {
    const m = MISSING[what];
    return (
      <Notice
        icon={<SearchX className="size-5" />}
        title={m.title}
        actions={
          <>
            <FloorLink primary />
            {/* Not on a touch screen, which has no keys to press. */}
            <span className="text-[13px] text-ink-4 [@media(hover:none)]:hidden">
              or find it with <Kbd>{shortcutText("palette")}</Kbd>
            </span>
          </>
        }
      >
        {m.text}
      </Notice>
    );
  }
  return (
    <Notice
      icon={<CloudOff className="size-5" />}
      title={`Couldn't open this ${what}`}
      actions={
        <>
          <Button variant="primary" loading={retrying} onClick={onRetry}>
            Try again
          </Button>
          <FloorLink />
        </>
      }
    >
      {error instanceof Error ? error.message : "Something went wrong."}
    </Notice>
  );
}

/** Admin, for someone who isn't one of the instance's admins. */
export function AdminsOnly() {
  return (
    <Notice
      icon={<LockKeyhole className="size-5" />}
      title="Admin is for this instance's admins"
      actions={<FloorLink primary />}
    >
      They manage its people, workspaces, runners and settings here. Ask one of them for what you need.
    </Notice>
  );
}

function FloorLink({ primary }: { primary?: boolean }) {
  return (
    <Link to="/">
      <Button variant={primary ? "primary" : "ghost"}>Go to the Agent Floor</Button>
    </Link>
  );
}

function Notice({
  icon,
  title,
  actions,
  children,
}: {
  icon: ReactNode;
  title: string;
  actions: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="mx-auto flex max-w-xl animate-rise flex-col items-start px-6 pt-24 pb-16 max-sm:pt-16">
      <span className="flex size-11 items-center justify-center rounded-xl bg-raised text-ink-3 shadow-[inset_0_0_0_1px_var(--c-line)]">
        {icon}
      </span>
      <h1 className="mt-5 font-display text-[26px] leading-[1.15] font-medium tracking-[-0.018em] text-ink">
        {title}
      </h1>
      <p className="mt-2 text-[15px] leading-relaxed text-ink-3">{children}</p>
      <div className="mt-7 flex flex-wrap items-center gap-x-4 gap-y-3">{actions}</div>
    </div>
  );
}
