import { Link } from "@tanstack/react-router";

export function NotFound() {
  return (
    <div className="flex min-h-dvh flex-col items-center justify-center px-6 text-center">
      <p className="font-mono text-[13px] text-ink-4">404</p>
      <h1 className="mt-3 font-serif text-[34px] font-medium tracking-[-0.02em]">
        Nothing is being built here.
      </h1>
      <Link to="/" className="mt-6 text-[14px] text-accent hover:underline">
        Back to the floor
      </Link>
    </div>
  );
}
