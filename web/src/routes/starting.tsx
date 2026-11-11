import { Logo } from "@/components/ui";

/**
 * What the app shows while a page waits on the server for over a second (the router's pending
 * component), as pages do when the app starts and asks who is signed in: the mark, and what it
 * waits for, rather than a blank screen. The page takes its place, or the error page once the
 * server doesn't answer (router.tsx).
 */
export function Starting() {
  return (
    <div className="flex min-h-dvh animate-fade flex-col items-center justify-center gap-4 bg-canvas px-6 motion-reduce:animate-none">
      <Logo large />
      <p role="status" className="text-center text-[13px] text-ink-4">
        Waiting for fctl’s server to answer…
      </p>
    </div>
  );
}
