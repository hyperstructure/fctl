// A GitHub account's avatar, in a rounded square as GitHub shows
// organizations'.

import { useState } from "react";
import { GithubIcon } from "@/components/ui";

/**
 * The account's avatar, `size` pixels square. From avatars.githubusercontent.com, not
 * github.com/<login>.png: that redirects there, but the Content Security Policy
 * (public/_headers) doesn't let images load from github.com.
 */
export function githubAvatarUrl(account: string, size: number): string {
  return `https://avatars.githubusercontent.com/${encodeURIComponent(account)}?s=${size}`;
}

/** 32 pixels square; the GitHub mark when the avatar doesn't load. */
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
