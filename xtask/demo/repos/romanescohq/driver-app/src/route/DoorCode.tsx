import { useState } from "react";
import { Pressable, StyleSheet, Text } from "react-native";
import { colors, space } from "@/src/theme";
import type { Stop } from "./types";

/** The building's door code, hidden until the driver taps it at the door. */
export function DoorCode({ stop }: { stop: Stop }) {
  const [shown, setShown] = useState(false);
  if (!stop.doorCode) return null;
  return (
    <Pressable onPress={() => setShown(true)} style={styles.code}>
      <Text style={styles.text}>{shown ? stop.doorCode : "Door code: tap to show"}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  code: { padding: space.md, borderRadius: 10, backgroundColor: colors.raised },
  text: { fontSize: 20, fontVariant: ["tabular-nums"], color: colors.ink },
});
