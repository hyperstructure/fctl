import { router, useLocalSearchParams } from "expo-router";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { deliver } from "@/src/delivery/deliver";
import { DoorCode } from "@/src/route/DoorCode";
import { useRoute } from "@/src/route/useRoute";
import { colors, space } from "@/src/theme";

/** One stop: where to go, how to get in, and what happened. */
export default function StopScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const stop = useRoute().find((s) => s.id === id);
  if (!stop) return null;

  const delivered = async () => {
    await deliver(stop);
    router.back();
  };

  return (
    <View style={styles.screen}>
      <Text style={styles.address}>{stop.address}</Text>
      <Text style={styles.name}>{stop.customerName}</Text>
      <DoorCode stop={stop} />
      {stop.notes ? <Text style={styles.notes}>{stop.notes}</Text> : null}
      <View style={styles.actions}>
        <Pressable style={[styles.button, styles.primary]} onPress={delivered}>
          <Text style={styles.primaryLabel}>Delivered</Text>
        </Pressable>
        <Pressable style={styles.button} onPress={() => router.push(`/stop/${stop.id}/failed`)}>
          <Text style={styles.label}>Couldn't deliver</Text>
        </Pressable>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, padding: space.lg, gap: space.md, backgroundColor: colors.surface },
  address: { fontSize: 24, fontWeight: "600", color: colors.ink },
  name: { fontSize: 17, color: colors.ink3 },
  notes: { fontSize: 15, color: colors.ink },
  actions: { marginTop: "auto", gap: space.sm },
  button: { borderRadius: 12, paddingVertical: space.lg, alignItems: "center", backgroundColor: colors.raised },
  primary: { backgroundColor: colors.green },
  primaryLabel: { fontSize: 17, fontWeight: "600", color: colors.surface },
  label: { fontSize: 17, color: colors.ink },
});
