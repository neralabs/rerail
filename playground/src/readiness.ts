interface RequestOptions {
  signal?: AbortSignal | undefined;
  fetch?: typeof globalThis.fetch | undefined;
}

/** A daemon reply is JSON only once it is OK and says so; anything else names the URL and status, never a bare parse error. */
export async function daemonJson(
  url: string,
  timeoutMs: number,
  unavailable: string,
  options: RequestOptions = {},
): Promise<unknown> {
  let response: Response;
  const timeout = AbortSignal.timeout(timeoutMs);
  const signal = options.signal ? AbortSignal.any([options.signal, timeout]) : timeout;
  try {
    response = await (options.fetch ?? globalThis.fetch)(url, { signal });
  } catch (error) {
    throw new Error(`${unavailable}: ${url} did not answer`, { cause: error });
  }
  const contentType = response.headers.get("content-type") ?? "";
  const text = await response.text();
  if (!response.ok || !contentType.startsWith("application/json"))
    throw new Error(
      `${unavailable}: ${url} answered ${response.status} ${contentType || "without a content type"}: ${text.slice(0, 200)}`,
    );
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`${unavailable}: ${url} answered ${response.status} with malformed JSON`, { cause: error });
  }
}
