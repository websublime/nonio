export const http = {
  get(url: string) {
    return fetch(url).then((r) => r.json());
  },
};
