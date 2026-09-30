// Thumbnail cache shared by every Thumb. Keys never contain a password: `withPw` only remembers which
// paths were rendered with one, so their entries can be looked up without keeping the password itself.
const MAX = 300;
const cache = new Map<string, string>(); // path|page|width -> data URL, oldest first
const withPw = new Set<string>();

const keyOf = (path: string, page: number, w: number) => `${path}|${page}|${w}`;

export const getThumb = (path: string, page: number, w: number) => cache.get(keyOf(path, page, w));

export function putThumb(path: string, page: number, w: number, url: string, hasPassword: boolean) {
  if (hasPassword) withPw.add(path);
  const k = keyOf(path, page, w);
  cache.delete(k);
  cache.set(k, url);
  if (cache.size > MAX) cache.delete(cache.keys().next().value as string);
}

export function clearThumbCache() {
  cache.clear();
  withPw.clear();
}
