const API = process.env.PLATFORM_API ?? "http://localhost:8080";

export async function nextDelivery(subscriptionId: string): Promise<{ date: string }> {
  const res = await fetch(`${API}/subscriptions/${subscriptionId}/next-delivery`);
  return res.json();
}

export async function skipDelivery(subscriptionId: string, date: string) {
  await fetch(`${API}/subscriptions/${subscriptionId}/deliveries/${date}/skip`, { method: "POST" });
}
