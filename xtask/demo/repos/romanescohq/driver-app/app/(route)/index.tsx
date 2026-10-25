import { router } from "expo-router";
import { FlatList, StyleSheet, Text, View } from "react-native";
import { StopRow } from "@/src/route/StopRow";
import { nextStop, useRoute } from "@/src/route/useRoute";
import { colors, space } from "@/src/theme";

/** Today's round: every stop, in order. */
export default function RouteScreen() {
  const stops = useRoute();
  const next = nextStop(stops);
  const left = stops.filter((s) => s.status === "pending").length;

  return (
    <View style={styles.screen}>
      <Text style={styles.summary}>
        {left === 0 ? "Every stop is done" : `${left} of ${stops.length} stops left`}
        {next ? ` · next: ${next.address}` : ""}
      </Text>
      <FlatList
        data={stops}
        keyExtractor={(s) => s.id}
        renderItem={({ item }) => <StopRow stop={item} onPress={(s) => router.push(`/stop/${s.id}`)} />}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: colors.surface },
  summary: { padding: space.lg, fontSize: 15, color: colors.ink3 },
});
