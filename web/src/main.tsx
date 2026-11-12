import "./styles/app.css";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { VerifyDialog } from "@/components/two-factor/verify";
import { Toaster } from "@/components/ui";
import { onCaughtError, onUncaughtError, startAppLog } from "@/lib/app-log";
import { linksInPlace } from "@/lib/history";
import { queryClient } from "@/lib/queries";
import { syncShimmers } from "@/lib/shimmer";
import { watchViewport } from "@/lib/viewport";
import { stopPinchZoom } from "@/lib/zoom";
import { router } from "./router";

// First: the log keeps how this run goes, for Settings → App log.
startAppLog();
stopPinchZoom();
syncShimmers();
watchViewport();
// On an iPhone's or iPad's Home Screen, links load their pages without adding to the history.
linksInPlace();

// Sessions have one view since the classic one went: browsers that kept which of the two to show
// (fctl:session-view) forget it.
try {
  localStorage.removeItem("fctl:session-view");
} catch {}

// Makes the app installable and shows an offline page (public/sw.js). Not in
// development, where it would outlive the dev server on this origin.
if (import.meta.env.PROD && "serviceWorker" in navigator) {
  addEventListener("load", () => navigator.serviceWorker.register("/sw.js").catch(() => {}));
}

const root = document.getElementById("root");
if (root) {
  // An error nothing catches takes the whole app down, leaving the screen blank: the page loads
  // again (lib/app-log.ts). The error page catches the rest, and the log keeps them all.
  createRoot(root, { onUncaughtError, onCaughtError }).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
        <VerifyDialog />
        <Toaster />
      </QueryClientProvider>
    </StrictMode>,
  );
}
