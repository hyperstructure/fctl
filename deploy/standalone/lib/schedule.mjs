// Cloudflare runs the Worker's cron trigger (the hourly housekeeping in
// crates/fctl-control/src/housekeeping.rs); Miniflare doesn't, so this
// does. The Worker's trigger is `M * * * *`, minute M of every hour, which
// is all this understands: anything else runs hourly at minute 0.

const HOUR = 3_600_000;

/** The minute of `M * * * *`. */
export function minuteOf(cron) {
  const m = /^(\d{1,2}) \* \* \* \*$/.exec(cron?.trim() ?? "");
  const minute = m ? Number(m[1]) : Number.NaN;
  return minute < 60 ? minute : undefined;
}

/** When `minute` next comes round after `now` (milliseconds since the epoch, UTC as cron is). */
export function nextRun(now, minute) {
  const hour = Math.floor(now / HOUR) * HOUR;
  const at = hour + minute * 60_000;
  return at > now ? at : at + HOUR;
}

/**
 * Calls `run(cron, scheduledTime)` at every occurrence of `cron` until
 * `stop()`. A run that is still going when the next one is due delays it.
 */
export function hourly(cron, run, { now = Date.now } = {}) {
  const minute = minuteOf(cron) ?? 0;
  let timer;
  let stopped = false;
  let last = 0;
  const plan = () => {
    if (stopped) return;
    // After the last occurrence too: a timer that fired a little early
    // mustn't run that one again.
    const at = nextRun(Math.max(now(), last), minute);
    timer = setTimeout(
      async () => {
        last = at;
        await run(cron, at);
        plan();
      },
      Math.max(1_000, at - now()),
    );
    timer.unref?.();
  };
  plan();
  return {
    stop() {
      stopped = true;
      clearTimeout(timer);
    },
  };
}
