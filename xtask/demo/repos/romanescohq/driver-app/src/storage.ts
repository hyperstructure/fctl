import { createMMKV } from "react-native-mmkv";

/** What the app keeps on the phone between launches: the day's route and the outbox. */
export const storage = createMMKV({ id: "driver" });
