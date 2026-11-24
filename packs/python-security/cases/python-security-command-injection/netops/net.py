import subprocess


def ping(host):
    result = subprocess.run(f"ping -c1 {host}", shell=True, capture_output=True)  # caught
    return result.returncode == 0


def is_valid_host(host):
    return host.replace(".", "").isalnum()


def checked_ping(host):
    if not is_valid_host(host):
        raise ValueError(host)
    return subprocess.run(f"ping -c1 {host}", shell=True).returncode == 0
