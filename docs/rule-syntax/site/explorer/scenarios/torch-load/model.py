import torch


def restore(path):
    return torch.load(path)


def restore_on_cpu(path):
    return torch.load(path, map_location="cpu")


def restore_weights(path):
    return torch.load(path, weights_only=True)
