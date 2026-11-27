import { http } from './http';

export function getOrders() {
  return http.get('/api/orders');
}
