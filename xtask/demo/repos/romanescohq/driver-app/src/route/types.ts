/** A stop on the day's route: one delivery to one address. */
export type Stop = {
  id: string;
  /** Its place in the round, from 1. */
  sequence: number;
  address: string;
  postcode: string;
  customerName: string;
  /** The building's door code: shown only at the door, and never logged. */
  doorCode: string | null;
  notes: string | null;
  /** The customer's delivery window, as ISO times. */
  window: { from: string; to: string };
  /** When the van gets there at the pace it's going, as an ISO time. */
  eta: string;
  boxes: number;
  cold: boolean;
  status: "pending" | "delivered" | "failed";
  /** What of the stop may go in a log. */
  toLogSafe(): LogSafeStop;
};

/** A stop as logs may have it: no name, address or door code. */
export type LogSafeStop = Pick<Stop, "id" | "sequence" | "postcode" | "status">;

export function logSafe(stop: Omit<Stop, "toLogSafe">): LogSafeStop {
  return { id: stop.id, sequence: stop.sequence, postcode: stop.postcode, status: stop.status };
}
