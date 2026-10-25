import { log } from "@/src/log";
import type { Stop } from "@/src/route/types";
import { outbox } from "./outbox";

/** Marks a stop delivered: at once when the phone is online, else as soon as it is. */
export async function deliver(stop: Stop): Promise<void> {
  log.info("delivered", { stop: stop.toLogSafe() });
  await outbox.push({ kind: "delivered", stopId: stop.id });
}

/** A delivery that didn't happen, why, and a photo of the damage when there is one. */
export async function fail(stop: Stop, reason: string, photo?: string): Promise<void> {
  log.info("failed", { stop: stop.toLogSafe(), reason });
  await outbox.push({ kind: "failed", stopId: stop.id, reason, photo });
}
