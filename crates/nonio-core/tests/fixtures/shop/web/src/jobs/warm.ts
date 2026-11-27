import { GET } from '../../../next/app/api/stock/route';

export async function warm() {
  return GET();
}
