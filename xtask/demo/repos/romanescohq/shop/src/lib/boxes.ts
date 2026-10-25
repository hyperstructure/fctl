import type { Item } from "@/components/BoxItem";

const API = process.env.PLATFORM_API ?? "http://localhost:8080";

/** What a box holds this week, as the platform has it. */
export async function boxItems(slug: string): Promise<Item[]> {
  const res = await fetch(`${API}/boxes/${slug}/items`, { next: { revalidate: 300 } });
  return res.json();
}
