import subprocess


def exec_cmd(cmd, sudo_user=None, **kwargs):
    """Run a command; Modoboa's shape: shell is set through kwargs."""
    kwargs["shell"] = True
    if sudo_user is not None:
        cmd = f"sudo -u {sudo_user} {cmd}"
    process = subprocess.Popen(cmd, stdout=subprocess.PIPE, **kwargs)  # caught
    return process.communicate()


def run_opts(cmd, **kw):
    kw.setdefault("shell", True)
    return subprocess.check_call(cmd, **kw)  # caught


def run_list(argv, **kw):
    # kw never turns the shell on; argv is a list with a fixed program.
    return subprocess.run(argv, **kw)
