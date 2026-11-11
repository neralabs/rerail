import os
import stat


def publish(path):
    os.chmod(path, 0o777)


def make_executable(path):
    os.chmod(path, stat.S_IRWXU | stat.S_IXOTH)


def lock_down(path):
    os.chmod(path, 0o600)


def private_dir(path):
    os.chmod(path, stat.S_IRWXU)
