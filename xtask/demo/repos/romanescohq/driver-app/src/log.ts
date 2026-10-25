import * as Sentry from "@sentry/react-native";

type Data = Record<string, unknown>;

/** The app's log: the console in development, and Sentry's breadcrumbs in builds. */
export const log = {
  info(message: string, data?: Data) {
    if (__DEV__) console.log(message, data ?? "");
    Sentry.addBreadcrumb({ level: "info", message, data });
  },
  warn(message: string, data?: Data) {
    if (__DEV__) console.warn(message, data ?? "");
    Sentry.addBreadcrumb({ level: "warning", message, data });
  },
  error(message: string, error: unknown, data?: Data) {
    if (__DEV__) console.error(message, error, data ?? "");
    Sentry.captureException(error, { extra: { message, ...data } });
  },
};
