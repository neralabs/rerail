try:
    run()
except Exception:
    pass

try:
    run()
except Exception:  # nosec
    pass

try:
    run()
except Exception:  # nosec B110
    pass

try:
    run()
except Exception:  # nosec: B110
    pass

try:
    run()
except Exception:  # nosec try_except_pass
    pass

try:
    run()
except Exception:  # nosec B101
    pass
