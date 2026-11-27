import axios from 'axios';
import { api, external } from './http';

export function apiGet(path: string) {
  return api.get(path);
}

export function payInvoice(path: string) {
  return axios.post(`/api/invoices${path}`);
}

export function mirrorGet(path: string) {
  return external.get(path);
}
