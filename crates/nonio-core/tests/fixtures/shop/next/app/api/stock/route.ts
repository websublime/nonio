export async function GET() {
  const orders = await fetch('/api/orders');
  return Response.json({ stock: 3, orders: await orders.json() });
}
