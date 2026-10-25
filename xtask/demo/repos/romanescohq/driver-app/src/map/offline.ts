import Mapbox from "@rnmapbox/maps";
import { log } from "@/src/log";

type Bounds = [[number, number], [number, number]];

/** The day's route area, downloaded at the depot so that the map works in basements and tunnels. */
export async function downloadRouteArea(routeId: string, bounds: Bounds) {
  const name = `route-${routeId}`;
  const existing = await Mapbox.offlineManager.getPack(name);
  if (existing) return existing;
  return Mapbox.offlineManager.createPack(
    { name, styleURL: Mapbox.StyleURL.Street, bounds, minZoom: 11, maxZoom: 17 },
    (_pack, status) => log.info("offline tiles", { route: routeId, percentage: status.percentage }),
    (_pack, error) => log.warn("offline tiles failed", { route: routeId, error: error.message }),
  );
}

/** Earlier days' areas go, so that the phone doesn't fill up. */
export async function deleteOldAreas(keep: string) {
  for (const pack of await Mapbox.offlineManager.getPacks()) {
    if (pack.name !== `route-${keep}`) await Mapbox.offlineManager.deletePack(pack.name);
  }
}
