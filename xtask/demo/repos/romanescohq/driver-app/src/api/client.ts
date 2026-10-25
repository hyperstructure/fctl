import * as SecureStore from "expo-secure-store";
import { logSafe, type Stop } from "@/src/route/types";

const base = process.env.EXPO_PUBLIC_API_URL ?? "https://api.staging.romanesco.farm";

async function call(path: string, init: RequestInit = {}): Promise<Response> {
  const token = await SecureStore.getItemAsync("token");
  const res = await fetch(`${base}${path}`, {
    ...init,
    headers: { ...init.headers, authorization: `Bearer ${token}` },
  });
  if (!res.ok) throw new Error(`${init.method ?? "GET"} ${path}: ${res.status}`);
  return res;
}

/** Today's route for the driver who is signed in, in order. */
export async function fetchRoute(): Promise<Stop[]> {
  const res = await call("/v1/driver/route");
  const { stops } = (await res.json()) as { stops: Omit<Stop, "toLogSafe">[] };
  return stops.map((s) => ({ ...s, toLogSafe: () => logSafe(s) }));
}

/** Tells the platform what happened at a stop. */
export async function markStop(id: string, status: "delivered" | "failed", reason?: string) {
  await call(`/v1/driver/stops/${id}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ status, reason }),
  });
}

/** A photo taken at a stop, for the platform to keep with it. */
export async function uploadStopPhoto(id: string, file: { uri: string; type: string }, kind: "damage") {
  const form = new FormData();
  form.append("kind", kind);
  form.append("photo", { uri: file.uri, type: file.type, name: `${kind}.jpg` } as unknown as Blob);
  await call(`/v1/driver/stops/${id}/photos`, { method: "POST", body: form });
}
