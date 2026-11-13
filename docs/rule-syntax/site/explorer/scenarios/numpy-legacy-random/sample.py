import numpy as np


def noise(n):
    return np.random.rand(n)


def pick(values):
    np.random.seed(0)
    return np.random.choice(values)


def noise_ok(n, seed=0):
    rng = np.random.default_rng(seed)
    return rng.random(n)
