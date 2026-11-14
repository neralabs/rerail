def has(config, key):
    return key in config.keys()


def names(config):
    return [k for k in config.keys()]


def has_direct(config, key):
    return key in config


def has_value(config, value):
    return value in config.values()
