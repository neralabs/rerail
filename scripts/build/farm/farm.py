"""The build farm: one compiler cache for every Rerail worktree, and the idle Macs for its misses.

See README.md beside this file. Two entry points use this module:

- `rustc-wrapper`, cargo's `RUSTC_WRAPPER` in Rerail's worktrees, sends each compile through the farm's
  sccache (its own server and cache, `PORT`).
- `farm-rustc`, the compiler that sccache runs on a miss, compiles here or on another Mac.
"""
import fcntl
import hashlib
import json
import os
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


def same_in_every_checkout(args, out_dir, crate):
    """Linker arguments that keep a proc macro's dylib free of its target directory's path. sccache never caches
    a proc macro, but every crate that uses one hashes its dylib: with the checkout's path in it (its
    install name, and the debug map's paths to the rlibs it links), each target directory's crates
    above it had different keys."""
    types = set()
    for t in values(args, "--crate-type"):
        types.update(t.split(","))
    if "proc-macro" not in types or sys.platform != "darwin" or not out_dir:
        return []
    # `<target>/<profile>/deps`: the debug map's paths start at the profile's directory.
    target = os.path.dirname(os.path.dirname(os.path.abspath(out_dir)))
    name = f"lib{crate}{codegen(args, 'extra-filename') or ''}.dylib"
    return ["-C", f"link-arg=-Wl,-install_name,@rpath/{name}", "-C", f"link-arg=-Wl,-oso_prefix,{target}/"]


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


# ---- a miss


def hosts():
    try:
        return json.load(open(HOSTS))
    except (OSError, ValueError):
        return []


def capacity(host):
    """How many compiles `host` takes now: its cores less its load, read at most every 20 s."""
    path = state("load", host["name"])
    try:
        st = os.stat(path)
        if time.time() - st.st_mtime < 20:
            return int(open(path).read() or 0)
    except (OSError, ValueError):
        pass
    # One compile reads the load; the others take the last value meanwhile (none at first).
    lock = Lock(path + ".lock", blocking=False)
    if not lock.held:
        lock.release()
        try:
            return int(open(path).read() or 0)
        except (OSError, ValueError):
            return 0
    try:
        out = subprocess.run(ssh(host["name"], "sysctl -n vm.loadavg"), capture_output=True, text=True, timeout=5)
        load = float(out.stdout.split()[1]) if out.returncode == 0 else None
    except (subprocess.TimeoutExpired, ValueError, IndexError):
        load = None
    finally:
        lock.release()
    # The load counts the farm's own compiles there: only the rest is the host's own work.
    free = 0 if load is None else max(0, int(host["cores"] - max(0.0, load - held(host)) + 0.5))
    open(path + ".tmp", "w").write(str(free))
    os.replace(path + ".tmp", path)
    return free


# The ssh connection a compile's calls go over: one master per slot, since sshd serves only so many
# sessions over one connection (MaxSessions, 10 by default) and refuses or stalls the rest.
CHANNEL = ["load"]


def ssh(host, command):
    control = state("ssh", f"{host}.{CHANNEL[0]}")
    return ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", "-o", "ServerAliveInterval=5",
            "-o", "ServerAliveCountMax=3", "-o", "ControlMaster=auto", "-o", f"ControlPath={control}",
            "-o", "ControlPersist=10m", host, command]


def held(host):
    """How many of `host`'s slots compiles hold now."""
    n = 0
    for i in range(host["cores"]):
        lock = Lock(state("slots", f"{host['name']}.{i}"), blocking=False)
        n += not lock.held
        lock.release()
    return n


def remote_slot():
    if os.environ.get("RERAIL_FARM_LOCAL"):
        return None, None
    for host in hosts():
        if not host.get("enabled", True):
            continue
        for i in range(capacity(host)):
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
    """Copies `names` (relative to `root`) to the same paths on `host`."""
    if not names:
        return True
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
            return False
        finally:
            tar.stdout.close()
        return tar.wait() == 0 and put.returncode == 0
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
    """Compiles on `host`. False when anything but the compile itself failed, or the compile failed:
    the caller compiles here then, and only that run's output is shown."""
    name = host["name"]
    cwd = os.getcwd()
    out_dir = os.path.abspath(value(args, "--out-dir"))
    crate, extra = value(args, "--crate-name"), codegen(args, "extra-filename") or ""
    checkout = checkout_of(cwd)
    # The crate's sources: its checkout's files, or (a dependency's) its own directory.
    if checkout and not sync(name, checkout, tracked(checkout), since):
        return False
    if not checkout and not sync(name, cwd, [n for n in walk(cwd)], since):
        return False
    deps = [f for f in os.listdir(out_dir) if f.endswith(".rmeta") or (f.startswith("lib") and f.endswith(".dylib"))]
    externs = [e.split("=", 1)[1] for e in values(args, "--extern") if "=" in e]
    deps += [os.path.basename(e) for e in externs if os.path.dirname(os.path.abspath(e)) == out_dir]
    if not sync(name, out_dir, sorted(set(deps)), since):
        return False
    for e in externs:
        d = os.path.dirname(os.path.abspath(e))
        if d != out_dir and not sync(name, d, [os.path.basename(e)], since):
            return False
    if env.get("OUT_DIR") and not sync(name, env["OUT_DIR"], list(walk(env["OUT_DIR"])), since):
        return False
    # The native libraries it names (`-L native=dir`, from build scripts): rustc bundles a static one
    # into the rlib.
    for d in values(args, "-L"):
        d = os.path.abspath(d.split("=", 1)[-1])
        if d != out_dir and os.path.isdir(d) and not sync(name, d, [f for f in os.listdir(d) if os.path.isfile(os.path.join(d, f))], since):
            return False
    request = {"cwd": cwd, "argv": [rustc] + args, "dirs": [out_dir],
               "env": {k: v for k, v in env.items() if k not in LOCAL_ONLY and not k.startswith(("SCCACHE_", "RERAIL_FARM_"))}}
    started = time.time()
    try:
        run = subprocess.run(ssh(name, os.path.join(SHARE, "remote-rustc")), input=json.dumps(request).encode(),
                             capture_output=True, timeout=COMPILE_SECONDS)
    except subprocess.TimeoutExpired:
        log(f"{crate}: {name} timed out")
        return False
    if run.returncode != 0:
        log(f"{crate}: {name} failed ({run.returncode}): {run.stderr[-400:]!r}")
        return False
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
        log(f"{crate}: outputs from {name} timed out")
        return False
    finally:
        get.stdout.close()
    if get.wait() != 0 or put.returncode != 0:
        log(f"{crate}: outputs from {name} not copied: {get.stderr.read()[-300:]!r} {put.stderr[-300:]!r} "
            f"(rustc said {run.stderr[-300:]!r})")
        return False
    sys.stdout.buffer.write(run.stdout)
    sys.stderr.buffer.write(run.stderr)
    log(f"{crate}: compiled on {name} in {time.time() - started:.1f} s")
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
    # A crate this checkout compiled incrementally before compiles here again, from that state.
    if inc and has_incremental(inc, crate):
        log(f"{crate}: missed, incremental here")
        local(rustc, args + ["-C", f"incremental={inc}"])
    if remote_eligible(args):
        host, lock = remote_slot()
        if host:
            try:
                if remote(host, rustc, args, env, since):
                    sys.exit(0)
            finally:
                lock.release()
    log(f"{crate}: missed, here")
    local(rustc, args + (["-C", f"incremental={inc}"] if inc else []))


def has_incremental(root, crate):
    """Whether `crate` has incremental state under cargo's `-C incremental` directory, which every crate
    of the profile shares: rustc keeps each crate's in a `<crate>-<hash>` directory of its own."""
    try:
        return any(d.startswith(crate + "-") for d in os.listdir(root))
    except OSError:
        return False


def local(rustc, args):
    """Compiles here, in one of this machine's slots: held by this process, since a lock does not
    survive an exec."""
    lock = local_slot()
    try:
        code = subprocess.run([rustc] + args).returncode
    finally:
        lock.release()
    sys.exit(code)
