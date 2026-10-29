def f(xs):
    for x in xs:
        try:
            g(x)
        except ValueError:
            h("a # b")  # trailing
    for x in xs:
        try:
            g(x)
        except ValueError:
            s = 'it''s # x'
            # own line
            # another
    for x in xs:
        try:
            g(x)
        except ValueError:
            h(x)  # don't "quote"
    for x in xs:
        try:
            g(x)
        except ValueError:
            h(x); y = 1  # c
    for x in xs:
        try:
            g(x)
        except ValueError:
            h(
                x,  # inner
            )
            # after
    for x in xs:
        try:
            g(x)
        except ValueError:
            t = """
            # not a comment
            """
    for x in xs:
        try:
            g(x)
        except ValueError:
            if x:
                pass  # deep
            # tail

    for x in xs:
        try:
            g(x)
        except ValueError:
            y = 1 + \
                2  # cont
    for x in xs:
        try:
            g(x)
        except ValueError: pass  # one line
    for x in xs:
        try:
            g(x)
        except ValueError:
            h(r"\"#")  # raw
