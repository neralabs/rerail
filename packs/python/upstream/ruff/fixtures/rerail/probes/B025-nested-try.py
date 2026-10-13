try:
    a()
except ValueError:
    pass
except Exception:
    try:
        b()
    except ValueError:
        pass
