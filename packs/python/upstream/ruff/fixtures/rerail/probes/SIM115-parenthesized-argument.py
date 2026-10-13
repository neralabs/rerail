import contextlib
with contextlib.ExitStack() as stack:
    f = stack.enter_context((open("f")))
