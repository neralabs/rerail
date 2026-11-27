def validator(*fields):
    def wrap(fn):
        return fn

    return wrap
