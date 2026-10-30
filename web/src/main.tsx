import "./styles/app.css";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { VerifyDialog } from "@/components/two-factor/verify";
import { Toaster } from "@/components/ui";
import { queryClient } from "@/lib/queries";
import { syncShimmers } from "@/lib/shimmer";
import { watchViewport } from "@/lib/viewport";
import { stopPinchZoom } from "@/lib/zoom";
import { router } from "./router";

stopPinchZoom();
syncShimmers();
watchViewport();

// Makes the app installable and shows an offline page (public/sw.js). Not in
// development, where it would outlive the dev server on this origin.
if (import.meta.env.PROD && "serviceWorker" in navigator) {
  addEventListener("load", () => navigator.serviceWorker.register("/sw.js").catch(() => {}));
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
        <VerifyDialog />
        <Toaster />
      </QueryClientProvider>
    </StrictMode>,
  );
}
