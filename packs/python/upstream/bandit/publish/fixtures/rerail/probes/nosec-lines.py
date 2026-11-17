import requests
requests.get(
    "https://example.com",  # nosec B113
)
requests.get(
    "https://example.com",
)  # nosec
requests.get(
    "https://example.com",  # nosec B101
)
try:
    run()
except Exception:
    pass  # nosec B110
