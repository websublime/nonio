export function request(path: string, init?: RequestInit) {
  return fetch(`/api${path}`, init);
}

export const version = 1;
