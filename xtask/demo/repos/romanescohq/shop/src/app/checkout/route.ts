export async function POST(req: Request) {
  const order = await req.json();
  return Response.json({
    ...order,
    depot: "sto-1",
  });
}
