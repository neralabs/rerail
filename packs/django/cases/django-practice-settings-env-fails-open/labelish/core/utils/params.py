import os


def get_env(name, default=None, is_bool=False):
    for env_key in ["LABEL_STUDIO_" + name, name]:
        value = os.environ.get(env_key)
        if value is not None:
            if is_bool:
                return value.lower() in ("1", "true", "yes")
            return value
    return default


def get_bool_env(key, default):
    return get_env(key, default, is_bool=True)


def get_env_list(key, default=None):
    value = get_env(key)
    if not value:
        if default is None:
            return []
        return default
    return value.split(",")
