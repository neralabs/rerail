for item in items:
    try:
        run(item)
    except Exception:
        continue

for item in items:
    try:
        run(item)
    except Exception:  # nosec
        continue

for item in items:
    try:
        run(item)
    except Exception:  # nosec B112
        continue

for item in items:
    try:
        run(item)
    except Exception:  # nosec: B112
        continue

for item in items:
    try:
        run(item)
    except Exception:  # nosec try_except_continue
        continue

for item in items:
    try:
        run(item)
    except Exception:  # nosec B101
        continue
