import os
import subprocess


def test_ping(request):
    os.system(f"ping -c1 {request.GET['host']}")
    subprocess.run("echo " + request.POST["x"], shell=True)
