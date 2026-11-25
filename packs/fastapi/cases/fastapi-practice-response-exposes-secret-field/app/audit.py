def field_serializer(*names):
    def wrap(fn):
        return fn

    return wrap
