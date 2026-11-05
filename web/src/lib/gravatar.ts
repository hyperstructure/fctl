// Gravatar: people who have no picture of their own show theirs, which the
// control plane copies (Me.gravatar). The app says so once, when one
// becomes their picture.

import type * as G from "@/gen";

/** localStorage: the Gravatar picture (its address) this browser last said was theirs. */
const SHOWN = "fctl:gravatar-shown";

/** The address of `me`'s picture, when it is their Gravatar. */
export function gravatarPicture(me?: G.Me | null): string | undefined {
  return me?.avatar_source === "gravatar" ? me.user.avatar_url : undefined;
}

/**
 * Whether to tell someone that `picture`, their Gravatar, is their picture now: once for each one
 * fctl copies (its address changes with it), in each browser. Remembers that it did.
 */
export function gravatarNews(
  picture: string | undefined,
  storage: Pick<Storage, "getItem" | "setItem"> = localStorage,
): boolean {
  if (!picture) return false;
  try {
    if (storage.getItem(SHOWN) === picture) return false;
    storage.setItem(SHOWN, picture);
  } catch {
    // Without storage (private browsing), it says so on each page load.
  }
  return true;
}
