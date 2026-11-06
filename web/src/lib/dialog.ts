// A <dialog>'s own events, apart from those of the dialogs opened from it.

import type { SyntheticEvent } from "react";

/**
 * `handler`, for a <dialog>'s own `close` or `cancel` event only. The
 * browser keeps those on the dialog they happen to, but React hands them on
 * to the components around it too, through portals: without this, a dialog
 * opened from a sheet would close the sheet along with it, and the sheet
 * would take the dialog's Escape for its own.
 */
export function own<E extends SyntheticEvent>(handler: (e: E) => void): (e: E) => void {
  return (e) => {
    if (e.target === e.currentTarget) handler(e);
  };
}
