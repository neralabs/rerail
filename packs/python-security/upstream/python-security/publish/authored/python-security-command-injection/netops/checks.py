import ipaddress
import os
import subprocess

BLOCKED = {"localhost", "127.0.0.1"}
ZONES = {"eu", "us"}


def run_healthcheck(host):
    # Named like a check, but it is the command: the host reaches the shell.
    return subprocess.check_output(f"ping -c1 {host}", shell=True)  # caught


def check_disk(path):
    # Starts with a checking verb, and still runs the command itself: no check.
    return subprocess.check_output(f"df -h {path}", shell=True)  # caught


def checkout_branch(name):
    subprocess.run(f"git checkout {name}", shell=True)  # caught


def is_valid_host(host):
    try:
        ipaddress.ip_address(host)
    except ValueError:
        return False
    return True


def view_health(request):
    host = request.GET["host"]
    return run_healthcheck(host)


def view_disk(request):
    path = request.GET["path"]
    check_disk(path)
    os.system(f"du -sh {path}")  # caught


def view_checkout(request):
    branch = request.POST["branch"]
    checkout_branch(branch)


def view_validated(request):
    host = request.GET["host"]
    if not is_valid_host(host):
        raise ValueError(host)
    os.system(f"traceroute {host}")


def view_blocklist(request):
    host = request.GET["host"]
    if host in BLOCKED:
        raise ValueError(host)
    os.system(f"ping -c1 {host}")  # caught


def view_allowlist(request):
    zone = request.GET["zone"]
    if zone not in ZONES:
        raise ValueError(zone)
    os.system(f"dig @{zone}.ns.example.com")
