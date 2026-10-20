// On phones and tablets the app keeps the screen's size: it doesn't zoom in or
// out. index.html's viewport tag says so (user-scalable=no), and browsers on
// Android keep to it, but Safari on iOS and iPadOS lets people pinch anyway.
// Its pinches arrive as gesture events, which only Safari sends; cancelling
// them stops the zoom.

const PINCH = ["gesturestart", "gesturechange"];

/** Stops pinch zoom on a touch screen (not on a Mac's trackpad). Returns what undoes it. */
export function stopPinchZoom(target: EventTarget = document, touch = navigator.maxTouchPoints > 0) {
  if (!touch) return () => {};
  const cancel = (e: Event) => e.preventDefault();
  for (const type of PINCH) target.addEventListener(type, cancel, { passive: false });
  return () => {
    for (const type of PINCH) target.removeEventListener(type, cancel);
  };
}
