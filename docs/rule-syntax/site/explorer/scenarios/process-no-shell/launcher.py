import os
import subprocess


def replace_with(program, *args):
    os.execv(program, [program, *args])


def spawn(program):
    return os.spawnl(os.P_NOWAIT, program, program)


def run(program):
    return subprocess.run([program], check=True)
