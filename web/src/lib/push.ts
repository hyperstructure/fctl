// Notifications in this browser: the service worker (public/sw.js) and its
// Web Push subscription, which the control plane encrypts notifications for.
// A subscription belongs to the account that turned it on; signing out ends it.

import type * as G from "@/gen";
import { api } from "./api";
import { browserName } from "./format";

/** The account that turned notifications on in this browser. */
const OWNER = "fctl-push-user";

/**
 * Whether this browser can show notifications. "install": an iPhone or iPad,
 * where only fctl added to the Home Screen can.
 */
export type PushSupport = "supported" | "install" | "unsupported";

export function pushSupport(): PushSupport {
  if ("serviceWorker" in navigator && "PushManager" in window && "Notification" in window) return "supported";
  const ios =
    /iPhone|iPad|iPod/.test(navigator.userAgent) ||
    (navigator.userAgent.includes("Macintosh") && navigator.maxTouchPoints > 1);
  return ios ? "install" : "unsupported";
}

export function permission(): NotificationPermission {
  return "Notification" in window ? Notification.permission : "denied";
}

/** base64url (a VAPID key) as bytes. */
export function base64urlBytes(text: string): Uint8Array<ArrayBuffer> {
  const b64 = text.replace(/-/g, "+").replace(/_/g, "/");
  const bin = atob(b64 + "=".repeat((4 - (b64.length % 4)) % 4));
  return Uint8Array.from(bin, (c) => c.charCodeAt(0));
}

/** Whether `sub` was made with the server key `publicKey` (base64url). */
export function madeWith(sub: PushSubscription, publicKey: string): boolean {
  const key = sub.options.applicationServerKey;
  if (!key) return false;
  const have = new Uint8Array(key);
  const want = base64urlBytes(publicKey);
  return have.length === want.length && have.every((b, i) => b === want[i]);
}

function device(sub: PushSubscription): G.AddPushDeviceRequest {
  const keys = sub.toJSON().keys ?? {};
  return {
    endpoint: sub.endpoint,
    keys: { p256dh: keys.p256dh ?? "", auth: keys.auth ?? "" },
    name: browserName(navigator.userAgent),
  };
}

async function registration(): Promise<ServiceWorkerRegistration> {
  await navigator.serviceWorker.register("/sw.js");
  return navigator.serviceWorker.ready;
}

async function subscribe(reg: ServiceWorkerRegistration, publicKey: string) {
  return reg.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: base64urlBytes(publicKey),
  });
}

/**
 * Turns notifications on in this browser. Call it from a click: it asks for
 * permission first.
 */
export async function enablePush(publicKey: string, userId: string): Promise<G.PushDevice> {
  const allowed = await Notification.requestPermission();
  if (allowed !== "granted") {
    throw new Error(
      allowed === "denied"
        ? "Notifications are blocked for this site. Allow them in your browser's site settings, then turn them on here."
        : "Notifications were not allowed.",
    );
  }
  const reg = await registration();
  let sub = await reg.pushManager.getSubscription();
  if (sub && !madeWith(sub, publicKey)) {
    await sub.unsubscribe();
    sub = null;
  }
  sub ??= await subscribe(reg, publicKey).catch((e: unknown) => {
    // Brave turns push off until "Use Google services for push messaging" is on.
    const brave =
      "brave" in navigator ? " In Brave, turn on Google services for push messaging in its settings." : "";
    throw new Error(
      `This browser's push service didn't take the subscription (${e instanceof Error ? e.message : e}).${brave}`,
    );
  });
  const added = await api.addPushDevice(device(sub));
  localStorage.setItem(OWNER, userId);
  return added;
}

/** Turns notifications off in this browser, and for `deviceId` on the server. */
export async function disablePush(deviceId?: string): Promise<void> {
  await forgetPush();
  if (deviceId) await api.removePushDevice(deviceId);
}

/**
 * Signing out: this browser stops getting notifications. The server deletes
 * its subscription with the sign-in.
 */
export async function forgetPush(): Promise<void> {
  localStorage.removeItem(OWNER);
  if (pushSupport() !== "supported") return;
  try {
    const reg = await navigator.serviceWorker.getRegistration();
    const sub = await reg?.pushManager.getSubscription();
    await sub?.unsubscribe();
  } catch {
    // Nothing to end.
  }
}

/**
 * At start-up: keeps the server's copy of this browser's subscription
 * current (a new sign-in, renewed keys, a new server key), and ends one that
 * another account turned on.
 */
export async function syncPush(userId: string): Promise<void> {
  if (pushSupport() !== "supported" || Notification.permission !== "granted") return;
  const existing = await navigator.serviceWorker.getRegistration();
  let sub = await existing?.pushManager.getSubscription();
  if (!sub) return;
  if (localStorage.getItem(OWNER) !== userId) {
    await sub.unsubscribe();
    return;
  }
  const { public_key } = await api.notifications();
  if (!public_key) return;
  // Registering again also checks for a newer service worker.
  const reg = await registration();
  if (!madeWith(sub, public_key)) {
    await sub.unsubscribe();
    sub = await subscribe(reg, public_key);
  }
  await api.addPushDevice(device(sub));
}

/** Follows clicks on notifications: the service worker asks an open window to show a page. */
export function onPushOpen(navigate: (url: string) => void): () => void {
  if (!("serviceWorker" in navigator)) return () => {};
  const listener = (e: MessageEvent) => {
    const data = e.data as { type?: string; url?: string } | null;
    if (data?.type === "open" && typeof data.url === "string" && data.url.startsWith("/")) navigate(data.url);
  };
  navigator.serviceWorker.addEventListener("message", listener);
  navigator.serviceWorker.startMessages();
  return () => navigator.serviceWorker.removeEventListener("message", listener);
}
