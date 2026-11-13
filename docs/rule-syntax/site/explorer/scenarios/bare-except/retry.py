import time


def retry(action, attempts=3):
    for attempt in range(attempts):
        try:
            return action()
        except:
            time.sleep(2 ** attempt)
    try:
        return action()
    except Exception:
        return None
