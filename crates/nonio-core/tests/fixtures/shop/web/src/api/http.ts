import axios from 'axios';

export const api = axios.create({ baseURL: '/api' });

export const external = axios.create({ baseURL: import.meta.env.VITE_API_URL });
