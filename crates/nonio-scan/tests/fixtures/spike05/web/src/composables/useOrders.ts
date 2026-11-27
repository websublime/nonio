import { ref } from 'vue';
import { getOrders } from '../api/client';

export function useOrders() {
  const orders = ref([]);
  async function load() {
    orders.value = await getOrders();
  }
  return { orders, load };
}
