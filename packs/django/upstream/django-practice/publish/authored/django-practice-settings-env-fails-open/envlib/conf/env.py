import os


def get_list(text):
    return [item.strip() for item in text.split(",") if item]


def get_bool_from_env(name, default_value):
    value = os.environ.get(name)
    if value is None:
        return default_value
    return value.lower() in ("true", "1")
