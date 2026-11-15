"""The build farm: one compiler cache for every Rerail worktree, and the idle Macs for its misses.

See README.md beside this file. Two entry points use this module:

- `rustc-wrapper`, cargo's `RUSTC_WRAPPER` in Rerail's worktrees, sends each compile through the farm's
  sccache (its own server and cache, `PORT`).
- `farm-rustc`, the compiler that sccache runs on a miss, compiles here or on another Mac.
- `farm-cc` and `farm-c++`, build scripts' C compilers (`CC`, `CXX`), go through the same sccache.
"""
import concurrent.futures
import fcntl
import hashlib
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time

HOME = os.path.expanduser("~")
SHARE = os.path.join(HOME, ".local/share/rerail-farm")
CONFIG = os.path.join(HOME, ".config/rerail-farm")
STATE = os.path.join(HOME, "Library/Caches/rerail-farm")
SCCACHE = os.path.join(SHARE, "bin/sccache")
SCCACHE_CONF = os.path.join(CONFIG, "sccache.toml")
HOSTS = os.path.join(CONFIG, "hosts.json")
PORT = "4227"
CACHE_SIZE_GB = 60
# A workspace crate that missed this recently is being edited here: it compiles incrementally, uncached.
HOT_SECONDS = 2 * 3600
# A host nobody has touched for this long is idle: it takes a compile per core.
USER_IDLE_SECONDS = 300
# A remote step that takes longer than this has stalled: the compile runs here instead.
COPY_SECONDS = 120
COMPILE_SECONDS = 900
# What a crate's build reads as a path must be hashed as it is, not with its checkout stripped.
VERBATIM = "CARGO_MANIFEST_DIR,CARGO_MANIFEST_PATH,RERAIL_FIXTURES_DIR,RERAIL_ORACLES_DIR"
# Variables a remote compile must not see: this machine's jobserver, terminal and farm plumbing.
LOCAL_ONLY = ("MAKEFLAGS", "MFLAGS", "CARGO_MAKEFLAGS", "TMPDIR", "PWD", "OLDPWD", "SSH_AUTH_SOCK",
              "SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY", "TERM_SESSION_ID", "__CF_USER_TEXT_ENCODING")


def log(message):
    if os.environ.get("RERAIL_FARM_LOG"):
        with open(os.environ["RERAIL_FARM_LOG"], "a") as f:
            f.write(f"{time.strftime('%H:%M:%S')} {os.getpid()} {message}\n")


# ---- rustc's arguments


def value(args, flag):
    """The value of `--flag v` or `--flag=v`, the last one given."""
    found = None
    for i, a in enumerate(args):
        if a == flag and i + 1 < len(args):
            found = args[i + 1]
        elif a.startswith(flag + "="):
            found = a[len(flag) + 1:]
    return found


def values(args, flag):
    out = []
    for i, a in enumerate(args):
        if a == flag and i + 1 < len(args):
            out.append(args[i + 1])
        elif a.startswith(flag + "="):
            out.append(a[len(flag) + 1:])
    return out


def codegen(args, key):
    """The value of `-C key=v` or `-Ckey=v`."""
    found = None
    for i, a in enumerate(args):
        opt = args[i + 1] if a == "-C" and i + 1 < len(args) else a[2:] if a.startswith("-C") and len(a) > 2 else None
        if opt is not None and opt.startswith(key + "="):
            found = opt[len(key) + 1:]
    return found


def without_incremental(args):
    """`args` without `-C incremental=…`, and that directory."""
    out, inc, skip = [], None, False
    for i, a in enumerate(args):
        if skip:
            skip = False
            continue
        if a == "-C" and i + 1 < len(args) and args[i + 1].startswith("incremental="):
            inc, skip = args[i + 1][len("incremental="):], True
            continue
        if a.startswith("-Cincremental="):
            inc = a[len("-Cincremental="):]
            continue
        out.append(a)
    return out, inc


def emits(args):
    kinds = set()
    for e in values(args, "--emit"):
        kinds.update(k.split("=")[0] for k in e.split(","))
    return kinds or {"link"}


def remote_eligible(args):
    """A library's compile: its outputs are files in `--out-dir`, and it links nothing."""
    types = set()
    for t in values(args, "--crate-type"):
        types.update(t.split(","))
    return types <= {"lib", "rlib"} and "--test" not in args and value(args, "--out-dir") is not None


# ---- places


def checkout_of(path):
    """The checkout (worktree) holding `path`: the nearest directory above it with a `.git`."""
    path = os.path.abspath(path)
    while path != "/":
        if os.path.exists(os.path.join(path, ".git")):
            return path
        path = os.path.dirname(path)
    return None


def key(text):
    return hashlib.sha1(text.encode()).hexdigest()[:16]


def state(*parts):
    path = os.path.join(STATE, *parts)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    return path


class Lock:
    def __init__(self, path, blocking=True):
        self.f = open(path, "a+")
        self.held = True
        try:
            fcntl.flock(self.f, fcntl.LOCK_EX | (0 if blocking else fcntl.LOCK_NB))
        except BlockingIOError:
            self.held = False

    def release(self):
        if self.held:
            fcntl.flock(self.f, fcntl.LOCK_UN)
            self.held = False
        self.f.close()


# ---- the farm's sccache


def sccache_env(env):
    env.pop("SCCACHE_DIR", None)
    env.update(
        SCCACHE_SERVER_PORT=PORT,
        SCCACHE_CONF=SCCACHE_CONF,
        SCCACHE_BASEDIRS_REMAP_PATHS="1",
        SCCACHE_BASEDIRS_VERBATIM_ENV_DEPS=VERBATIM,
        # Proc macros, build scripts and the engine's cdylib are cached too (`same_in_every_checkout`).
        SCCACHE_CACHE_LINKED="1",
        # The server restarts when a new worktree joins (`join`): a compile it drops runs here instead.
        SCCACHE_IGNORE_SERVER_IO_ERROR="1",
    )
    return env


def basedirs():
    try:
        return set(json.load(open(SCCACHE_CONF + ".basedirs")))
    except (OSError, ValueError):
        return set()


def join(checkout):
    """Makes `checkout` one of the server's basedirs: every worktree of its repository is, so the server
    restarts once per new worktree, not once per checkout."""
    if checkout in basedirs():
        return
    lock = Lock(state("sccache.lock"))
    try:
        if checkout in basedirs():
            return
        listed = subprocess.run(["git", "-C", checkout, "worktree", "list", "--porcelain"], capture_output=True, text=True).stdout
        dirs = basedirs() | {checkout} | {line[len("worktree "):] for line in listed.splitlines() if line.startswith("worktree ")}
        write_conf(sorted(dirs))
        env = sccache_env(dict(os.environ))
        subprocess.run([SCCACHE, "--stop-server"], env=env, capture_output=True)
        log(f"sccache: {len(dirs)} basedirs, restarted for {checkout}")
    finally:
        lock.release()


def write_conf(dirs):
    os.makedirs(CONFIG, exist_ok=True)
    cache = os.path.join(STATE, "sccache")
    lines = ["# Written by scripts/build/farm (farm.py): the farm's sccache, for Rerail's worktrees only.",
             "basedirs = [" + ", ".join(json.dumps(d) for d in dirs) + "]", "", "[cache.disk]",
             f"dir = {json.dumps(cache)}", f"size = {CACHE_SIZE_GB * 1024 ** 3}", ""]
    tmp = SCCACHE_CONF + ".tmp"
    open(tmp, "w").write("\n".join(lines))
    os.replace(tmp, SCCACHE_CONF)
    json.dump(dirs, open(SCCACHE_CONF + ".basedirs.tmp", "w"))
    os.replace(SCCACHE_CONF + ".basedirs.tmp", SCCACHE_CONF + ".basedirs")


def shim(rustc):
    """The compiler sccache is given for `rustc`: a directory of its own per rustc and its mtime, since
    sccache keeps what it learnt of a compiler (version, sysroot) by the compiler's path."""
    real = os.path.realpath(rustc)
    stamp = f"{real}:{os.stat(real).st_mtime_ns}"
    d = os.path.join(SHARE, "shims", key(stamp))
    path = os.path.join(d, "rustc")
    if not os.path.exists(path):
        os.makedirs(d, exist_ok=True)
        open(os.path.join(d, "real.tmp"), "w").write(rustc)
        os.replace(os.path.join(d, "real.tmp"), os.path.join(d, "real"))
        try:
            os.symlink(os.path.join(SHARE, "farm-rustc"), path)
        except FileExistsError:
            pass
    return path


def hot_marker(out_dir, crate, extra):
    return state("hot", key(out_dir), crate + (extra or ""))


LINKED = {"proc-macro", "cdylib", "dylib", "bin"}


def same_in_every_checkout(args, out_dir, crate):
    """Arguments that keep what rustc links (a proc macro's dylib, a build script, the engine's cdylib)
    free of its checkout's and target directory's paths: its install name and the debug map's paths
    to the objects it links. The farm's sccache caches these (`SCCACHE_CACHE_LINKED`), and every crate
    that uses a proc macro hashes its dylib. A cached one's debug map names objects of the target
    directory that compiled it: elsewhere, its own code has no line tables."""
    types = set()
    for t in values(args, "--crate-type"):
        types.update(t.split(","))
    if not types & LINKED or not out_dir:
        return []
    # `<target>/<profile>/deps`: the debug map's paths start at the profile's directory.
    target = os.path.dirname(os.path.dirname(os.path.abspath(out_dir)))
    # Paths in its metadata and its crate hash, such as a build script's OUT_DIR that it includes.
    # rustc applies the last prefix that matches: the target directory may be inside the checkout.
    checkout = checkout_of(os.getcwd())
    out = ["--remap-path-prefix", f"{checkout}=/rerail-farm/checkout"] if checkout else []
    out += ["--remap-path-prefix", f"{target}=/rerail-farm/target"]
    if sys.platform == "darwin":
        # The path as a link-arg of its own (`link-arg=<path>`), where sccache strips a basedir from
        # its key: in `-Wl,-oso_prefix,<path>` it would not, and each checkout would key apart.
        out += ["-C", "link-arg=-Xlinker", "-C", "link-arg=-oso_prefix", "-C", "link-arg=-Xlinker", "-C", f"link-arg={target}/"]
        # A cdylib's file name has no hash suffix; a proc macro's and a dylib's has.
        name = f"lib{crate}.dylib" if "cdylib" in types else \
            f"lib{crate}{codegen(args, 'extra-filename') or ''}.dylib" if types & {"proc-macro", "dylib"} else None
        if name:
            out += ["-C", f"link-arg=-Wl,-install_name,@rpath/{name}"]
    return out


def wrapper(rustc, args):
    """cargo's RUSTC_WRAPPER."""
    env = dict(os.environ)
    crate = value(args, "--crate-name")
    if crate is None or not os.path.exists(SCCACHE):
        os.execv(rustc, [rustc] + args)
    out_dir = value(args, "--out-dir")
    checkout = checkout_of(out_dir or os.getcwd()) or checkout_of(os.getcwd())
    if checkout:
        join(checkout)
    args = args + same_in_every_checkout(args, out_dir, crate)
    stripped, inc = without_incremental(args)
    if inc is not None:
        marker = hot_marker(out_dir or "", crate, codegen(args, "extra-filename"))
        try:
            hot = time.time() - os.stat(marker).st_mtime < HOT_SECONDS
        except OSError:
            hot = False
        if hot:
            log(f"{crate}: hot, incremental here")
            os.execv(rustc, [rustc] + args)
        env["RERAIL_FARM_INCREMENTAL"] = inc
        env["RERAIL_FARM_HOT"] = marker
        args = stripped
    # sccache hashes every CARGO_ variable; rustc reads neither of these, and each names a place.
    for k in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"):
        env.pop(k, None)
    sccache_env(env)
    os.execve(SCCACHE, [SCCACHE, shim(rustc)] + args, env)


def cc(argv0, args):
    """`farm-cc` and `farm-c++`, the C and C++ compilers of build scripts (`CC`, `CXX`): clang through
    the farm's sccache, so a build script's C (SQLite, tree-sitter's grammars, simdjson) is compiled
    once for every target directory and worktree, not once per target directory."""
    compiler = "/usr/bin/clang++" if os.path.basename(argv0).endswith("++") else "/usr/bin/clang"
    if not os.path.exists(SCCACHE):
        os.execv(compiler, [compiler] + args)
    checkout = checkout_of(os.getcwd()) or checkout_of(os.environ.get("OUT_DIR", ""))
    if checkout:
        join(checkout)
    env = sccache_env(dict(os.environ))
    for k in ("CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"):
        env.pop(k, None)
    os.execve(SCCACHE, [SCCACHE, compiler] + args, env)


# ---- a miss


# The local configuration (README.md, "Configuration"): never in git. Its keys, with their defaults:
DEFAULTS = {
    "hosts": [],
    # A host is used only while its disk has this much free, its round trip is this short and the
    # link carries this much: a host that dropped to WiFi or a busy network is no faster than here.
    "min_free_gb": 40,
    "max_rtt_ms": 150,
    "min_mbps": 8,
}
# How long a host's health is believed: an answer that it is usable, and the first wait after it was
# not (doubled each time it is still not, up to the last).
HEALTHY_SECONDS = 30
DOWN_SECONDS = (30, 1800)
# A host that answers, but over another link than its own (it fell back to WiFi), is tried again
# this often, not backed off: its own link may be back at any time (`recover`).
OFF_LINK_SECONDS = 60
# The throughput probe's size, and how often it is sent (it is the slowest part of the probe). The
# round trip is taken off its time: at 4 MB a session's start alone halved the rate seen.
PROBE_BYTES = 16 << 20
PROBE_SECONDS = 600


def config_path():
    """The configuration file read: `$RERAIL_FARM_CONFIG`, else `~/.config/rerail-farm/farm.json`, else
    the older `hosts.json` (a bare list of hosts); None when there is none."""
    path = os.environ.get("RERAIL_FARM_CONFIG") or os.path.join(CONFIG, "farm.json")
    return next((p for p in (path, HOSTS) if os.path.exists(p)), None)


def config():
    """The farm's configuration (`config_path`). No file: no hosts, and everything runs here."""
    for p in filter(None, [config_path()]):
        try:
            data = json.load(open(p))
        except (OSError, ValueError):
            continue
        if isinstance(data, list):
            data = {"hosts": data}
        return dict(DEFAULTS, **data)
    return dict(DEFAULTS)


def hosts():
    return [h for h in config()["hosts"] if h.get("name")]


def read_json(path, default):
    try:
        return json.load(open(path))
    except (OSError, ValueError):
        return default


def write_json(path, data):
    tmp = f"{path}.{os.getpid()}.tmp"
    json.dump(data, open(tmp, "w"))
    os.replace(tmp, path)


def mark_down(name, why):
    """Records that `name` failed in use (its agent stopped, a file could not be written there): it
    is not used until its back-off ends, and the next health check asks again."""
    path = state("health", name)
    h = read_json(path, {})
    failures = h.get("failures", 0) + 1
    wait = min(DOWN_SECONDS[0] * 2 ** (failures - 1), DOWN_SECONDS[1])
    h.update(ok=False, why=why, failures=failures, checked=time.time(), down_until=time.time() + wait)
    write_json(path, h)
    log(f"{name}: down for {wait} s: {why}")


# Prints the host's interface an ssh connection came in on: the one holding the address it was
# reached at (the third word of $SSH_CONNECTION; a link-local one carries its zone).
ARRIVAL = ("s=$(echo $SSH_CONNECTION | cut -d' ' -f3); ifconfig | awk -v s=\"$s\" "
           "'/^[a-z]/ {i=$1; sub(\":$\", \"\", i)} $2==s || index($2, s \"%\")==1 {print i; exit}'")


def link_of(host):
    """The interface of `host` its farm traffic must arrive on (`link`; `link_match`, with a leading
    `%`, in older configurations), or None: any."""
    return (host.get("link") or host.get("link_match") or "").lstrip("%") or None


def probe(host, cfg, fell_back=False, recovered=False):
    """Asks `host` how it is, over ssh: its idle time, free disk, NFS mounts, the address it sees this
    machine connect from, then the round trip and (every PROBE_SECONDS) the link's throughput.
    Returns the health record, `ok` with `why` when not."""
    name = host["name"]
    previous = read_json(state("health", name), {})
    h = {"checked": time.time(), "ok": False, "failures": previous.get("failures", 0),
         "mbps": previous.get("mbps"), "mbps_at": previous.get("mbps_at", 0)}
    if previous.get("route") != route(name):
        h["mbps"], h["mbps_at"] = None, 0  # another path: its throughput is not known
    script = ('echo "$SSH_CONNECTION"; df -k "$HOME" | tail -1;'
              " ioreg -c IOHIDSystem -d 4 | awk '/HIDIdleTime/ {print $NF; exit}'; echo \"$(" + ARRIVAL + ")\";"
              " mount -t nfs | sed 's/.* on \\(.*\\) (.*/\\1/'")
    try:
        out = subprocess.run(ssh(name, script, "health"), capture_output=True, text=True, timeout=10)
    except subprocess.TimeoutExpired:
        out = None
    if out is None or out.returncode != 0 or len(out.stdout.splitlines()) < 4:
        if route(name) and not fell_back:
            # The address that reached it over its link is gone (it was renumbered, or the link
            # is down): back to what its name resolves to.
            set_route(name, None)
            return probe(host, cfg, fell_back=True, recovered=recovered)
        h["why"] = "unreachable (off, asleep, or off the network)"
        return h
    lines = out.stdout.splitlines()
    client = h["client"] = (lines[0].split() or [""])[0]
    try:
        h["free_gb"] = int(lines[1].split()[3]) // (1 << 20)
        h["idle_s"] = int(lines[2]) / 1e9
    except (ValueError, IndexError):
        h["why"] = "its answer could not be read"
        return h
    arrival = h["arrival"] = lines[3].strip()
    h["mounts"] = [m for m in lines[4:] if m]
    started = time.time()
    try:
        subprocess.run(ssh(name, "true", "health"), capture_output=True, timeout=5)
        h["rtt_ms"] = round((time.time() - started) * 1000)
    except subprocess.TimeoutExpired:
        h["rtt_ms"] = 5000
    link = link_of(host)
    if time.time() - h["mbps_at"] > PROBE_SECONDS or (h["mbps"] or 0) < cfg["min_mbps"]:
        started = time.time()
        try:
            subprocess.run(ssh(name, "cat >/dev/null", "health"), input=os.urandom(PROBE_BYTES), capture_output=True, timeout=10)
            h["mbps"] = round(PROBE_BYTES / (1 << 20) / max(time.time() - started - h["rtt_ms"] / 1000, 0.01), 1)
        except subprocess.TimeoutExpired:
            h["mbps"] = 0
        h["mbps_at"] = time.time()
    if link and arrival != link and not recovered and recover(host, arrival):
        return probe(host, cfg, fell_back=fell_back, recovered=True)
    h["route"] = route(name)
    if link and arrival != link:
        h["why"] = f"not on its link: reached over its {arrival or '?'}, not {link}"
    elif h["free_gb"] < cfg["min_free_gb"]:
        h["why"] = f"{h['free_gb']} GB free, under {cfg['min_free_gb']}"
    elif h["rtt_ms"] > cfg["max_rtt_ms"]:
        h["why"] = f"a {h['rtt_ms']} ms round trip, over {cfg['max_rtt_ms']}"
    elif (h["mbps"] or 0) < cfg["min_mbps"]:
        h["why"] = f"{h['mbps']} MB/s, under {cfg['min_mbps']}"
    else:
        h["ok"], h["why"], h["failures"] = True, "", 0
    return h


def health(host, fresh=False):
    """`host`'s health: probed at most every HEALTHY_SECONDS while usable, and after its back-off
    ends while not (`mark_down`, or a failed probe). One caller probes; the others read the last
    record meanwhile. `fresh` probes now."""
    name = host["name"]
    path = state("health", name)
    h = read_json(path, None)
    now = time.time()
    if h is not None and not fresh:
        if h.get("ok") and now - h.get("checked", 0) < HEALTHY_SECONDS:
            return h
        if not h.get("ok") and now < h.get("down_until", 0):
            return h
    lock = Lock(path + ".lock", blocking=fresh)
    if not lock.held:
        lock.release()
        return h or {"ok": False, "why": "not probed yet"}
    try:
        h = probe(host, config())
        if not h["ok"] and h["why"].startswith("not on its link"):
            h["down_until"] = now + OFF_LINK_SECONDS
        elif not h["ok"]:
            h["failures"] = h.get("failures", 0) + 1
            h["down_until"] = now + min(DOWN_SECONDS[0] * 2 ** (h["failures"] - 1), DOWN_SECONDS[1])
        write_json(path, h)
        # `ship` reads what the host mounts (never copy under its NFS mounts); unknown stays unknown.
        if "mounts" in h:
            open(state("mounts", name) + ".tmp", "w").write("\n".join(h["mounts"]))
            os.replace(state("mounts", name) + ".tmp", state("mounts", name))
        log(f"{name}: {'usable' if h['ok'] else 'not usable: ' + h['why']} ({h.get('rtt_ms')} ms, {h.get('mbps')} MB/s, {h.get('free_gb')} GB free)")
        return h
    finally:
        lock.release()


def usable(kind=None):
    """The enabled hosts that are usable now, most cores first; `kind` ("mirror", "tests") keeps those
    the configuration names for it."""
    out = []
    for host in hosts():
        if not host.get("enabled", True) or (kind and not host.get(kind, kind == "tests" and host.get("mirror"))):
            continue
        if health(host).get("ok"):
            out.append(host)
    return sorted(out, key=lambda h: -h.get("cores", 0))


def capacity(host):
    """How many compiles `host` takes now: none while it is not usable (`health`); one per core while
    nobody has used it for 5 minutes, half as many while someone does. Not its load average: macOS's
    background work (Photos' and Spotlight's analysis) keeps that high on an idle machine, and runs
    at a priority every compile preempts."""
    h = health(host)
    if not h.get("ok"):
        return 0
    return host["cores"] if h.get("idle_s", 0) > USER_IDLE_SECONDS else host["cores"] // 2


# The ssh connection a compile's calls go over: one master per slot, since sshd serves only so many
# sessions over one connection (MaxSessions, 10 by default) and refuses or stalls the rest.
CHANNEL = ["load"]


def route(host):
    """The address `recover` found reaching `host` over its own link, or None: its name, as ssh
    resolves it (over mDNS, whichever interface answers first)."""
    return read_json(state("route", host), {}).get("address")


def set_route(host, address, alias=None):
    """Records the address every ssh to `host` uses from now on (None: its name again), and closes
    the connections that went the other way, so nothing keeps using the slow link."""
    if address == route(host):
        return
    if address:
        write_json(state("route", host), {"address": address, "alias": alias})
    else:
        try:
            os.remove(state("route", host))
        except FileNotFoundError:
            pass
    close_masters(host)
    log(f"{host}: route {'over ' + address if address else 'by name'}")


def close_masters(host):
    """Ends every shared ssh connection to `host` (each channel's master); the next ssh connects
    anew. A command running over one keeps it until it ends."""
    directory = state("ssh", "x")[:-1]
    for f in os.listdir(directory):
        if f.startswith(host + "."):
            subprocess.run(["ssh", "-o", f"ControlPath={directory}{f}", "-O", "exit", host],
                           capture_output=True, timeout=5)


def ssh_alias(host):
    """What `host`'s key is recorded under (`HostKeyAlias`, else the address its name gives), so an
    ssh to another of its addresses checks the same key."""
    g = dict(line.split(" ", 1) for line in subprocess.run(["ssh", "-G", host], capture_output=True, text=True)
             .stdout.splitlines() if " " in line)
    return g.get("hostkeyalias") or g.get("hostname") or host


def ssh(host, command, channel=None, address=None, alias=None):
    """The ssh command running `command` on `host`, over the channel's shared connection: to its
    route when it has one (or to `address`, unshared, to try one; its key checked as `alias`'s)."""
    r = read_json(state("route", host), {}) if address is None else {"address": address, "alias": alias}
    via = []
    if r.get("address"):
        via = ["-o", "HostName=" + r["address"].replace("%", "%%"), "-o", f"HostKeyAlias={r.get('alias') or ssh_alias(host)}"]
    if address is not None:
        shared = ["-o", "ControlPath=none"]
    else:
        # Each route has its own masters: one made over another link is never reused.
        control = state("ssh", f"{host}.{key(r.get('address') or '')[:6]}.{channel or CHANNEL[0]}")
        shared = ["-o", "ControlMaster=auto", "-o", f"ControlPath={control}", "-o", "ControlPersist=10m"]
    return ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", "-o", "ServerAliveInterval=5",
            "-o", "ServerAliveCountMax=3"] + via + shared + [host, command]


def recover(host, arrival):
    """Finds a way back to `host` over its own link (`link_of`), when it answers over another (its
    `arrival` interface: it fell back to WiFi). Over the connection that works it lists its
    addresses on that link; each is tried (in parallel, 3 s to connect), and the first that arrives
    there becomes its route. False when none does: the link is down, and the next probe
    (OFF_LINK_SECONDS) tries again."""
    name, link = host["name"], link_of(host)
    try:
        out = subprocess.run(ssh(name, f"ifconfig {shlex.quote(link)}", "health"), capture_output=True, text=True, timeout=10)
    except subprocess.TimeoutExpired:
        return False
    v4, v6 = [], []
    for words in (line.split() for line in out.stdout.splitlines()):
        if len(words) > 1 and words[0] == "inet":
            v4.append(words[1])
        elif len(words) > 1 and words[0] == "inet6" and words[1].startswith("fe80"):
            # This machine's end of the link is most often named as the host's is.
            v6.append(words[1].split("%")[0] + "%" + link)
    candidates = v4 + v6
    if not candidates:
        return False
    alias = ssh_alias(name)

    def attempt(address):
        try:
            r = subprocess.run(ssh(name, ARRIVAL, address=address, alias=alias), capture_output=True, text=True, timeout=6)
        except subprocess.TimeoutExpired:
            return None
        return address if r.returncode == 0 and r.stdout.strip() == link else None

    with concurrent.futures.ThreadPoolExecutor(len(candidates)) as pool:
        found = next((a for a in pool.map(attempt, candidates) if a), None)
    if found:
        set_route(name, found, alias)
    return bool(found)


def on_host(script):
    """The command that runs this farm's `script` (remote-rustc, farm-agent) on a host: its text goes
    with the command, so a host needs nothing installed and a cleaned one still works."""
    here = os.path.join(os.path.dirname(os.path.realpath(__file__)), script)
    return "exec /usr/bin/python3 -u -c " + shlex.quote(open(here).read())


def remote_slot():
    """A free compile slot on a host, and its lock. Slots are taken in turn across the hosts (each
    one's first, then each one's second, ...), so every host works, not only the first listed."""
    if os.environ.get("RERAIL_FARM_LOCAL"):
        return None, None
    open_hosts = [(h, capacity(h)) for h in hosts() if h.get("enabled", True)]
    for i in range(max((c for _, c in open_hosts), default=0)):
        for host, cap in open_hosts:
            if i >= cap:
                continue
            lock = Lock(state("slots", f"{host['name']}.{i}"), blocking=False)
            if lock.held:
                CHANNEL[0] = str(i)
                return host, lock
            lock.release()
    return None, None


def local_slot():
    n = os.cpu_count() or 8
    while True:
        for i in range(n):
            lock = Lock(state("slots", f"local.{i}"), blocking=False)
            if lock.held:
                return lock
            lock.release()
        time.sleep(0.05)


def ship(host, root, names):
    """Copies `names` (relative to `root`) to the same paths on `host`. Never under a path the host
    mounts over NFS: that may be this machine's own directory, and the copy would write over it."""
    if not names:
        return True
    try:
        mounted = [m for m in open(state("mounts", host)).read().splitlines() if m]
    except OSError:
        mounted = None
    if mounted is None or any(root == m or root.startswith(m.rstrip("/") + "/") for m in mounted):
        log(f"copy to {host} {root} refused: the host mounts it, or its mounts are unknown")
        return False
    with tempfile.NamedTemporaryFile("w", delete=False) as f:
        f.write("\n".join(names) + "\n")
    try:
        tar = subprocess.Popen(["tar", "-cf", "-", "--no-xattrs", "--no-mac-metadata", "-C", root, "-T", f.name],
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        try:
            put = subprocess.run(ssh(host, f"mkdir -p '{root}' && tar -xf - -C '{root}'"), stdin=tar.stdout,
                                 capture_output=True, timeout=COPY_SECONDS)
        except subprocess.TimeoutExpired:
            tar.kill()
            log(f"copy to {host} {root} timed out")
            return False
        finally:
            tar.stdout.close()
        ok = tar.wait() == 0 and put.returncode == 0
        if not ok:
            log(f"copy to {host} {root} failed: tar {tar.returncode}, {put.stderr[-300:]!r}")
        return ok
    finally:
        os.unlink(f.name)


def sync(host, root, names, since):
    """Brings `host`'s copy of `names` under `root` up to date: only those changed since they were last
    sent. A sync that began after `since` (the compile's start) already has everything it needs."""
    base = state("sync", host, key(root))
    lock = Lock(base + ".lock")
    try:
        try:
            sent = json.load(open(base + ".json"))
        except (OSError, ValueError):
            sent = {"began": 0, "files": {}}
        if sent["began"] >= since and all(n in sent["files"] for n in names):
            return True
        began = time.time()
        changed, now = [], {}
        for n in names:
            try:
                st = os.stat(os.path.join(root, n))
            except OSError:
                continue
            now[n] = [st.st_mtime_ns, st.st_size]
            if sent["files"].get(n) != now[n]:
                changed.append(n)
        if not ship(host, root, changed):
            return False
        sent["files"].update(now)
        sent["began"] = began
        tmp = base + ".json.tmp"
        json.dump(sent, open(tmp, "w"))
        os.replace(tmp, base + ".json")
        log(f"sync {host} {root}: {len(changed)} of {len(names)} sent")
        return True
    finally:
        lock.release()


def tracked(checkout):
    out = subprocess.run(["git", "-C", checkout, "ls-files", "--cached", "--others", "--exclude-standard"],
                         capture_output=True, text=True)
    return [n for n in out.stdout.splitlines() if n]


def remote(host, rustc, args, env, since):
    """Compiles on `host`. False when anything failed, the compile included: the caller compiles here
    then, and only that run's output is shown. Logs why, and where the time went."""
    name = host["name"]
    cwd = os.getcwd()
    out_dir = os.path.abspath(value(args, "--out-dir"))
    crate, extra = value(args, "--crate-name"), codegen(args, "extra-filename") or ""
    checkout = checkout_of(cwd)
    times, mark = [], [time.time()]

    def lap(label):
        now = time.time()
        times.append(f"{label} {now - mark[0]:.2f}")
        mark[0] = now

    def give_up(why):
        log(f"{crate}: not on {name}: {why} ({', '.join(times)})")
        return False

    if host.get("shared"):
        return run_shared(host, rustc, args, env, cwd, out_dir, crate, times, lap, give_up)
    if host.get("mirror"):
        return run_mirror(host, rustc, args, env, cwd, out_dir, crate, since, lap, give_up)
    # The crate's sources: its checkout's files, or (a dependency's) its own directory.
    if checkout and not sync(name, checkout, tracked(checkout), since):
        return give_up("its checkout was not copied")
    if not checkout and not sync(name, cwd, list(walk(cwd)), since):
        return give_up("its sources were not copied")
    lap("sources")
    deps = [f for f in os.listdir(out_dir) if f.endswith(".rmeta") or (f.startswith("lib") and f.endswith(".dylib"))]
    externs = [e.split("=", 1)[1] for e in values(args, "--extern") if "=" in e]
    deps += [os.path.basename(e) for e in externs if os.path.dirname(os.path.abspath(e)) == out_dir]
    if not sync(name, out_dir, sorted(set(deps)), since):
        return give_up("its dependencies were not copied")
    for e in externs:
        d = os.path.dirname(os.path.abspath(e))
        if d != out_dir and not sync(name, d, [os.path.basename(e)], since):
            return give_up(f"{e} was not copied")
    if env.get("OUT_DIR") and not sync(name, env["OUT_DIR"], list(walk(env["OUT_DIR"])), since):
        return give_up("its OUT_DIR was not copied")
    # The native libraries it names (`-L native=dir`, from build scripts): rustc bundles a static one
    # into the rlib.
    # A system directory (Xcode's clang runtime) is the host's own.
    for d in values(args, "-L"):
        d = os.path.abspath(d.split("=", 1)[-1])
        if d != out_dir and d.startswith(HOME + "/") and os.path.isdir(d) and not sync(name, d, [f for f in os.listdir(d) if os.path.isfile(os.path.join(d, f))], since):
            return give_up(f"{d} was not copied")
    lap("deps")
    request = {"cwd": cwd, "argv": [rustc] + args, "dirs": [out_dir],
               "env": {k: v for k, v in env.items() if k not in LOCAL_ONLY and not k.startswith(("SCCACHE_", "RERAIL_FARM_"))}}
    try:
        run = subprocess.run(ssh(name, on_host("remote-rustc")), input=json.dumps(request).encode(),
                             capture_output=True, timeout=COMPILE_SECONDS)
    except subprocess.TimeoutExpired:
        return give_up("the compile timed out")
    lap("compile")
    if run.returncode != 0:
        return give_up(f"the compile failed ({run.returncode}): {run.stderr[-400:]!r}")
    kinds = emits(args)
    outputs = [f"{crate}{extra}.d"] * ("dep-info" in kinds) + [f"lib{crate}{extra}.rmeta"] * ("metadata" in kinds) \
        + [f"lib{crate}{extra}.rlib"] * ("link" in kinds)
    get = subprocess.Popen(ssh(name, f"tar -cf - --no-xattrs -C '{out_dir}' " + " ".join(outputs)),
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        put = subprocess.run(["tar", "-xmf", "-", "-C", out_dir], stdin=get.stdout, capture_output=True,
                             timeout=COPY_SECONDS)
    except subprocess.TimeoutExpired:
        get.kill()
        return give_up("its outputs timed out")
    finally:
        get.stdout.close()
    if get.wait() != 0 or put.returncode != 0:
        return give_up(f"its outputs were not copied: {get.stderr.read()[-300:]!r} {put.stderr[-300:]!r}")
    lap("outputs")
    sys.stdout.buffer.write(run.stdout)
    sys.stderr.buffer.write(run.stderr)
    log(f"{crate}: compiled on {name} ({', '.join(times)})")
    return True


READ_MARK = b"\0rerail-farm-read "


def run_shared(host, rustc, args, env, cwd, out_dir, crate, times, lap, give_up):
    """Compiles on a host that mounts this machine's checkouts and target directories at the same
    paths (`"shared": true`, README): nothing is copied either way. rustc reads its inputs and writes
    its outputs through the mount. What the host sees may lag what is here by its attribute cache: it
    waits until it sees each dependency and the dependency directory as they are here, and reports
    what it read (the dep-info's files), which must be what is here; else this compile runs here."""
    def here(path):
        try:
            st = os.stat(path)
            return [st.st_mtime_ns, st.st_size]
        except OSError:
            return None
    fresh = {}
    for e in values(args, "--extern"):
        if "=" in e:
            path = os.path.abspath(e.split("=", 1)[1])
            if here(path):
                fresh[path] = here(path)
    # rustc finds the dependencies of dependencies by listing this directory.
    for d in {out_dir} | {os.path.abspath(v.split("=", 1)[-1]) for v in values(args, "-L") if v.startswith("dependency=")}:
        if here(d):
            fresh[d] = [here(d)[0], None]
    crate_extra = f"{crate}{codegen(args, 'extra-filename') or ''}"
    request = {"cwd": cwd, "argv": [rustc] + args, "dirs": [out_dir], "fresh": fresh,
               "depinfo": os.path.join(out_dir, crate_extra + ".d") if "dep-info" in emits(args) else None,
               "env": {k: v for k, v in env.items() if k not in LOCAL_ONLY and not k.startswith(("SCCACHE_", "RERAIL_FARM_"))}}
    try:
        run = subprocess.run(ssh(host["name"], on_host("remote-rustc")), input=json.dumps(request).encode(),
                             capture_output=True, timeout=COMPILE_SECONDS)
    except subprocess.TimeoutExpired:
        return give_up("the compile timed out")
    lap("compile")
    if run.returncode == 75:
        return give_up(f"the host's view stayed stale: {run.stderr[-300:]!r}")
    if run.returncode != 0:
        return give_up(f"the compile failed ({run.returncode}): {run.stderr[-400:]!r}")
    stderr, _, report = run.stderr.rpartition(READ_MARK)
    if not _:
        stderr, report = run.stderr, b"{}"
    try:
        read = json.loads(report)
    except ValueError:
        read = {"?": None}
    stale = [p for p, seen in read.items() if seen != here(p)]
    if stale:
        return give_up(f"it read files that are not as they are here: {stale[:3]}")
    sys.stdout.buffer.write(run.stdout)
    sys.stderr.buffer.write(stderr)
    log(f"{crate}: compiled on {host['name']}, shared ({', '.join(times)}, {len(fresh)} fresh, {len(read)} read)")
    return True


def run_mirror(host, rustc, args, env, cwd, out_dir, crate, since, lap, give_up):
    """Compiles on a host through the live mirror (`"mirror": true`, README; mirror.py): the daemon
    here sends what changed of what the compile may read, then the compile, over one connection
    to the host; its outputs come back on it. What it read (the dep-info's files) must be what is
    here; else this compile runs here."""
    import mirror
    checkout = checkout_of(cwd)
    roots = [[checkout, "checkout"]] if checkout else [[cwd, "tree"]]
    roots.append([out_dir, "deps"])
    if env.get("OUT_DIR"):
        roots.append([os.path.abspath(env["OUT_DIR"]), "tree"])
    for d in values(args, "-L"):
        d = os.path.abspath(d.split("=", 1)[-1])
        if d != out_dir and d.startswith(HOME + "/") and os.path.isdir(d):
            roots.append([d, "top"])
    files = [os.path.abspath(e.split("=", 1)[1]) for e in values(args, "--extern") if "=" in e]
    extra = codegen(args, "extra-filename") or ""
    kinds = emits(args)
    outputs = [f"{crate}{extra}.d"] * ("dep-info" in kinds) + [f"lib{crate}{extra}.rmeta"] * ("metadata" in kinds) \
        + [f"lib{crate}{extra}.rlib"] * ("link" in kinds)
    run = {"cwd": cwd, "argv": [rustc] + args, "dirs": [out_dir],
           "outputs": [os.path.join(out_dir, o) for o in outputs],
           "depinfo": os.path.join(out_dir, f"{crate}{extra}.d") if "dep-info" in kinds else None,
           "env": {k: v for k, v in env.items() if k not in LOCAL_ONLY and not k.startswith(("SCCACHE_", "RERAIL_FARM_"))}}
    reply = mirror.compile({"host": host["name"], "since": since, "roots": roots, "files": files, "run": run})
    if reply is None:
        return give_up("the mirror daemon did not answer")
    t = reply.get("times", {})
    lap(f"sync {t.get('sync')} ({t.get('files')} files), compile {t.get('compile')}, mirror")
    if reply["code"] != 0:
        return give_up(f"the compile failed ({reply['code']}): {reply.get('why', '')} {reply['stderr'][-400:]!r}")

    def here(path):
        try:
            st = os.stat(path)
            return [st.st_mtime_ns, st.st_size]
        except OSError:
            return None
    stale = [p for p, seen in reply["read"].items() if seen != here(p)]
    if stale:
        return give_up(f"it read files that are not as they are here: {stale[:3]}")
    sys.stdout.buffer.write(reply["stdout"].encode("latin-1"))
    sys.stderr.buffer.write(reply["stderr"].encode("latin-1"))
    log(f"{crate}: compiled on {host['name']}, mirror ({t.get('sync')} s sync, {t.get('compile')} s compile, {len(reply['read'])} read)")
    return True


def walk(root):
    for d, _, files in os.walk(root):
        for f in files:
            yield os.path.relpath(os.path.join(d, f), root)


def farm_rustc(argv0, args):
    """The compiler sccache runs: rustc itself, here or on another Mac."""
    since = time.time()
    try:
        rustc = open(os.path.join(os.path.dirname(argv0), "real")).read().strip()
    except OSError:
        rustc = os.path.join(HOME, ".cargo/bin/rustc")
    env = dict(os.environ)
    crate = value(args, "--crate-name")
    # sccache's own calls (`--print file-names`, `--emit=dep-info` to list the sources) are not the
    # compile: they run here, as they are.
    if crate is None or any(a == "--print" or a.startswith("--print=") for a in args) or "link" not in emits(args):
        os.execv(rustc, [rustc] + args)
    inc = env.pop("RERAIL_FARM_INCREMENTAL", None)
    marker = env.pop("RERAIL_FARM_HOT", None)
    if marker:
        open(marker, "w").close()
    # A workspace crate that missed is being edited: it compiles here, incrementally, so its next
    # edit starts from that state. Sent to a host, it compiled whole there (20 s for logic), and
    # again whole here on the next edit, with no state to start from.
    if inc:
        log(f"{crate}: missed, incremental here")
        local(rustc, args + ["-C", f"incremental={inc}"])
    if remote_eligible(args):
        host, lock = remote_slot()
        if not host:
            log(f"{crate}: no host has a free slot")
        if host:
            try:
                if remote(host, rustc, args, env, since):
                    sys.exit(0)
            finally:
                lock.release()
    log(f"{crate}: missed, here")
    local(rustc, args)


def local(rustc, args):
    """Compiles here, in one of this machine's slots: held by this process, since a lock does not
    survive an exec."""
    lock = local_slot()
    try:
        code = subprocess.run([rustc] + args).returncode
    finally:
        lock.release()
    sys.exit(code)
