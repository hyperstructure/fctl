// A GitHub account's avatar, in a rounded square: the accounts the app is
// installed on are mostly organizations.

import { useState } from "react";
import { GithubIcon } from "@/components/ui";

/**
 * Its avatar, `size` pixels square, from avatars.githubusercontent.com: the Content Security
 * Policy (public/_headers) lets images load from there, and not from github.com, so not from
 * github.com/<login>.png, which redirects there.
 */
export function githubAvatarUrl(account: string, size: number): string {
  return `https://avatars.githubusercontent.com/${encodeURIComponent(account)}?s=${size}`;
}

/** 32 pixels square, or the GitHub mark when its avatar doesn't load. */
export function GithubAvatar({ account }: { account: string }) {
  const [broken, setBroken] = useState<string>();
  if (broken === account) {
    return (
      <span className="inline-flex size-8 shrink-0 items-center justify-center rounded-md bg-raised text-ink-4">
        <GithubIcon />
      </span>
    );
  }
  return (
    <img
      src={githubAvatarUrl(account, 64)}
      alt=""
      className="size-8 shrink-0 rounded-md bg-raised"
      referrerPolicy="no-referrer"
      onError={() => setBroken(account)}
    />
  );
}
