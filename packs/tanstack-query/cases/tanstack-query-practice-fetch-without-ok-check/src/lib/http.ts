export async function handleResponse<T>(res: Response): Promise<T> {
  if (!res.ok) {
    throw new Error(`Request failed with ${res.status}`);
  }
  return res.json() as Promise<T>;
}

export const fetcher = (url: string) => fetch(url).then((res) => res.json()); // caught

export async function fetchJson<T>(url: string, init?: RequestInit): Promise<T> {
  const res = await fetch(url, init);
  return handleResponse<T>(res);
}
