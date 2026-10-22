def f():
    try:
        def g():
            raise ValueError("x")

        class C:
            raise ValueError

        try:
            pass
        except KeyError:
            raise ValueError
        finally:
            raise ValueError
    except ValueError:
        pass
