import os
import shlex
import subprocess
from subprocess import check_output

from django.http import HttpResponse, JsonResponse

from netops import net
from netops.sysutils import exec_cmd, run_list, run_opts
from netops.utils import call_subprocess, sh

TOOLS = {"ping": ["ping", "-c1"], "trace": ["traceroute"]}
ALLOWED_ZONES = {"eu", "us"}


def ping(request):
    host = request.GET["host"]
    os.system(f"ping -c1 {host}")  # caught
    return HttpResponse("ok")


def lookup(request):
    out = subprocess.run("nslookup " + request.GET["d"], shell=True, capture_output=True)  # caught
    return HttpResponse(out.stdout)


def dig(request):
    name = request.POST.get("name", "")
    cmd = "dig +short %s" % name
    return HttpResponse(check_output(cmd, shell=True))  # caught


def raw(request):
    return HttpResponse(os.popen(request.POST["cmd"]).read())  # caught


def checkout(request):
    # galaxy_ng: a git ref spliced into a shell command, with a fixed cwd.
    ref = request.data["ref"]
    subprocess.run(args=f"git checkout {ref}", shell=True, cwd="/srv/repo")  # caught
    return HttpResponse("ok")


def camera(request):
    # motionEye: hand-written quotes are not quoting; the wrapper forwards shell=shell.
    device = request.POST["device"]
    cmd = f"v4l2-ctl -d '{device}' --list-formats-ext | grep -oE 'Size'"
    return HttpResponse(call_subprocess(cmd, shell=True))


def remote_ping(request):
    return JsonResponse({"up": net.ping(request.POST["host"])})


def dkim(request):
    # Modoboa: the wrapper sets kwargs["shell"] = True.
    domain = request.POST["domain"]
    exec_cmd(f"openssl genrsa -out /var/dkim/{domain}.pem 2048")
    return HttpResponse("ok")


def gzip(request):
    path = request.GET["path"]
    run_opts("gzip " + path)
    return HttpResponse("ok")


def default_shell(request):
    sh("du -sh " + request.GET["dir"])
    return HttpResponse("ok")


def scan(request):
    # pyLoad: the client picks the executable of a list-form call.
    scanner = request.POST["scanner"]
    subprocess.Popen([scanner, "--scan", "/srv/uploads"])  # caught
    return HttpResponse("ok")


def scan_executable(request):
    subprocess.run(["scan", "/srv/uploads"], executable=request.POST["exe"])  # caught
    return HttpResponse("ok")


def convert(request):
    upload = request.FILES["base_file"]
    os.system("ogr2ogr -f GeoJSON out.json " + upload.name)  # caught
    return HttpResponse("ok")


def concat(request):
    cmd = "tar -czf out.tgz "
    cmd += request.GET["dir"]
    subprocess.call(cmd, shell=True)  # caught
    return HttpResponse("ok")


def opts_dict(request):
    opts = {"shell": True, "check": False}
    subprocess.call("whois " + request.GET["domain"], **opts)  # caught
    return HttpResponse("ok")


def unpack(request):
    name = request.GET["archive"]
    subprocess.run(["sh", "-c", f"tar -xzf /srv/uploads/{name}"], check=True)  # caught
    script = ("/bin/bash", "-lc", "unzip " + name)
    subprocess.check_call(script)  # caught
    # After `sh -c CMD`, a further item is the shell's $0, not code.
    subprocess.run(["sh", "-c", "make all", request.GET["target"]])
    return HttpResponse("ok")


# ---- near misses: silent --------------------------------------------------------------------------------------------


def log(request):
    # List form, fixed program; the ref is a later argument (argument injection is out of scope).
    ref = request.GET["ref"]
    subprocess.run(["git", "log", ref])
    cmd = ["git", "show", ref]
    subprocess.run(cmd, check=True)
    subprocess.run(["git", "diff"] + [ref])
    run_list(["git", "fetch", request.GET["remote"]], check=True)
    net.checked_ping(request.GET["host"])
    return HttpResponse("ok")


def quoted(request):
    p = request.GET["p"]
    subprocess.run(f"ls {shlex.quote(p)}", shell=True)
    os.system("ls -l " + shlex.quote(request.GET["q"]))
    subprocess.run(shlex.join(["grep", request.GET["term"], "/var/log/app.log"]), shell=True)
    return HttpResponse("ok")


def literal(request):
    subprocess.run("ls -l | wc -l", shell=True)
    os.system("uptime")
    return HttpResponse("ok")


def side_channels(request):
    subprocess.run("sort", shell=True, input=request.body)
    subprocess.run("make build", shell=True, cwd=request.GET["dir"], env={"TARGET": request.GET["t"]})
    return HttpResponse("ok")


def count(request):
    n = int(request.GET["n"])
    os.system(f"seq {n}")
    return HttpResponse("ok")


def tool(request):
    argv = TOOLS[request.GET["tool"]]
    subprocess.run(argv + ["example.com"])
    other = TOOLS.get(request.GET["tool"])
    subprocess.run(other)
    return HttpResponse("ok")


def zone(request):
    zone = request.GET["zone"]
    if zone not in ALLOWED_ZONES:
        return HttpResponse(status=400)
    os.system(f"rndc reload {zone}")
    return HttpResponse("ok")


def no_shell_string(request):
    # No shell, a fixed program: the request value is an argument of the list.
    subprocess.run(["ping", "-c1", request.GET["host"]], shell=False)
    return HttpResponse("ok")


def runner(request):
    # A method named run on an object that is not subprocess.
    request.app_runner.run(request.GET["job"], shell=True)
    return HttpResponse("ok")
