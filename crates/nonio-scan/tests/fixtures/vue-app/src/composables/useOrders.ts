import { ref } from 'vue'
import { fetchOrders } from '../api/client'

export function useOrders() {
  const orders = ref<{ id: number; total: number }[]>([])
  async function load() {
    orders.value = await fetchOrders()
  }
  return { orders, load }
}
