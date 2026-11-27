export async function getOrder(id: number) {
  const res = await fetch(`/api/orders/${id}`);
  return res.json();
}
