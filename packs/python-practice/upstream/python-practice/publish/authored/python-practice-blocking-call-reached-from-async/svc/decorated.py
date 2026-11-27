from svc.services.wrapped import (compute, connect, fetch_alias, fetch_retry, flaky, git_sha, lookup, rates, slow_get,
                                  stacked, submitted_get, threaded_get, unknown_wrapped)


async def page(url, host):
    a = fetch_retry(url)  # caught
    b = fetch_alias(url)  # caught
    c = connect(host)  # caught
    d = git_sha()
    e = lookup(host)  # caught
    f = rates("EUR")  # caught
    g = stacked(url)  # caught
    h = unknown_wrapped(url)
    i = slow_get(url)  # caught
    j = flaky(url)  # caught
    k = submitted_get(url)
    m = await threaded_get(url)
    n = compute(2)  # caught
    return a, b, c, d, e, f, g, h, i, j, k, m, n
