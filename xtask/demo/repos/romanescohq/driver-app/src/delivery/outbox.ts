import { markStop, uploadStopPhoto } from "@/src/api/client";
import { storage } from "@/src/storage";

type Entry =
  | { kind: "delivered"; stopId: string }
  | { kind: "failed"; stopId: string; reason: string; photo?: string };

const KEY = "outbox";

function load(): Entry[] {
  return JSON.parse(storage.getString(KEY) ?? "[]") as Entry[];
}

function save(entries: Entry[]) {
  storage.set(KEY, JSON.stringify(entries));
}

/** What the platform hasn't heard yet, oldest first: kept on the phone until it's sent. */
export const outbox = {
  async push(entry: Entry) {
    save([...load(), entry]);
    await flush();
  },
};

/** Sends what it can, oldest first. It stops at the first failure, to try again once the phone is back online. */
export async function flush(): Promise<void> {
  const entries = load();
  while (entries.length > 0) {
    const entry = entries[0];
    try {
      if (entry.kind === "delivered") {
        await markStop(entry.stopId, "delivered");
      } else {
        await markStop(entry.stopId, "failed", entry.reason);
        if (entry.photo) await uploadStopPhoto(entry.stopId, { uri: entry.photo, type: "image/jpeg" }, "damage");
      }
    } catch {
      return;
    }
    entries.shift();
    save(entries);
  }
}
