import { Pressable, StyleSheet, Text, View } from "react-native";
import { colors, space } from "@/src/theme";
import type { Stop } from "./types";

type Props = { stop: Stop; onPress: (stop: Stop) => void };

/** A stop in the route list: its place in the round, the address and the boxes. */
export function StopRow({ stop, onPress }: Props) {
  const done = stop.status !== "pending";
  return (
    <Pressable onPress={() => onPress(stop)} style={[styles.row, done && styles.done]}>
      <Text style={styles.sequence}>{stop.sequence}</Text>
      <View style={styles.body}>
        <Text style={styles.address} numberOfLines={1}>
          {stop.address}
        </Text>
        <Text style={styles.meta}>
          {stop.boxes} {stop.boxes === 1 ? "box" : "boxes"}
          {stop.cold ? " · cold" : ""}
        </Text>
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", gap: space.md, paddingVertical: space.md, paddingHorizontal: space.lg },
  done: { opacity: 0.5 },
  sequence: { width: 28, fontSize: 17, fontVariant: ["tabular-nums"], color: colors.ink3 },
  body: { flex: 1, gap: 2 },
  address: { fontSize: 17, color: colors.ink },
  meta: { fontSize: 14, color: colors.ink3 },
});
