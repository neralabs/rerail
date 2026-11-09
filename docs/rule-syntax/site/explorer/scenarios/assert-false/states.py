def handle(state):
    if state == "open":
        return 1
    if state == "closed":
        return 0
    assert False, f"unknown state {state}"


def handle_ok(state):
    if state == "open":
        return 1
    raise AssertionError(f"unknown state {state}")
