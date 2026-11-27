import subprocess


def call_subprocess(args, shell=False, input=None):
    return subprocess.run(args, shell=shell, input=input, capture_output=True).stdout  # caught


def sh(cmd, shell=True):
    return subprocess.run(cmd, shell=shell).returncode  # caught
