import { useEffect, useState } from "react";
import { fetchRoute } from "@/src/api/client";
import { storage } from "@/src/storage";
import { logSafe, type Stop } from "./types";

function saved(): Stop[] {
  const raw = storage.getString("route");
  if (!raw) return [];
  return (JSON.parse(raw) as Omit<Stop, "toLogSafe">[]).map((s) => ({ ...s, toLogSafe: () => logSafe(s) }));
}

/** Today's route: what the phone has first, so that it works in basements, then the platform's. */
export function useRoute(): Stop[] {
  const [stops, setStops] = useState<Stop[]>(saved);
  useEffect(() => {
    let current = true;
    fetchRoute()
      .then((fresh) => {
        if (!current) return;
        storage.set("route", JSON.stringify(fresh));
        setStops(fresh);
      })
      .catch(() => {
        // Offline: keep what the phone has.
      });
    return () => {
      current = false;
    };
  }, []);
  return stops;
}

/** The stop to go to next: the first one still pending. */
export function nextStop(stops: Stop[]): Stop | undefined {
  return stops.find((s) => s.status === "pending");
}
