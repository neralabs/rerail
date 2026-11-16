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
import re
import shlex
import socket
import subprocess
import sys
import threading
import time

# The farm's version: the mirror daemon, its callers and the agents on the hosts are one version
# (mirror.py). `setup` installs only a version at least as new as the installed one.
VERSION = 15
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
# Where cargo keeps the crates it downloads: the same files at the same paths on every machine.
IN_PLACE = (os.path.join(HOME, ".cargo/registry/src"), os.path.join(HOME, ".cargo/git/checkouts"))
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


def stamp(path):
    try:
        st = os.stat(path)
        return st.st_mtime_ns, st.st_size
    except OSError:
        return None


def is_rerail(checkout):
    """Whether `checkout` is one of Rerail's: other projects keep plain sccache and their own C compiler."""
    try:
        return checkout is not None and 'members = ["native"' in open(os.path.join(checkout, "Cargo.toml")).read()
    except OSError:
        return False


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


TIDY_SECONDS = 15 * 60
# `diagnose --brief` after a clean runs at most this often per checkout: its findings change slowly.
DIAGNOSE_SECONDS = 3600


def target_of(out_dir):
    """The cargo target directory `out_dir` is in, or None."""
    d = os.path.realpath(out_dir or "")
    for _ in range(4):
        if os.path.isfile(os.path.join(d, "CACHEDIR.TAG")):
            return d
        d = os.path.dirname(d)
    return None


def tidy(target, checkout):
    """Starts `clean-target --wait` on `target`, at most once every 15 minutes per target directory:
    Rerail's worktrees together write terabytes of builds a day, and what the next build cannot use
    goes after the build ends (it waits for cargo's lock). Then, at most hourly, `diagnose --brief` on
    the checkout, whose findings `advise` prints at the next builds. Detached, at nice 19."""
    if os.environ.get("RERAIL_FARM_NO_TIDY"):
        return
    script = os.path.join(SHARE, "clean-target")
    if target is None or not os.path.exists(script):
        return
    stamp = state("tidy", key(target))
    try:
        if time.time() - os.stat(stamp).st_mtime < TIDY_SECONDS:
            return
    except OSError:
        pass
    # One starter per period: whoever creates the claim starts it.
    claim = stamp + f".{int(time.time() // TIDY_SECONDS)}"
    try:
        os.close(os.open(claim, os.O_CREAT | os.O_EXCL | os.O_WRONLY))
    except OSError:
        return
    open(stamp, "w").close()
    for old in os.listdir(os.path.dirname(stamp)):
        # The claims only (`<key>.<hour>`), not the log or the findings beside them.
        if re.fullmatch(re.escape(os.path.basename(stamp)) + r"\.\d+", old) and old != os.path.basename(claim):
            try:
                os.unlink(os.path.join(os.path.dirname(stamp), old))
            except OSError:
                pass
    findings = findings_path(checkout)
    with open(stamp + ".log", "a") as out:
        subprocess.Popen(["nice", "-n", "19", "/bin/sh", "-c",
                          '"$0" --wait "$1"; [ -n "$(find "$4" -mmin -"$5" 2>/dev/null)" ] || '
                          '{ "$2" --brief "$3" > "$4.tmp" 2>/dev/null && mv "$4.tmp" "$4"; }',
                          script, target, os.path.join(SHARE, "diagnose"), checkout, findings,
                          str(DIAGNOSE_SECONDS // 60)],
                         stdin=subprocess.DEVNULL, stdout=out, stderr=out, start_new_session=True, close_fds=True)


def findings_path(checkout):
    return state("findings", key(checkout))


ADVICE_SECONDS = 3 * 3600


# Options of `cargo test` and `cargo nextest run` that take a value (so the value is no test filter).
VALUED = {"-p", "--package", "--profile", "--features", "-F", "-j", "--jobs", "--target", "--target-dir",
          "--manifest-path", "--bin", "--test", "--example", "--bench", "--color", "--config", "-Z",
          "--message-format", "--partition", "--test-threads", "--retries", "--archive-file", "--cargo-profile"}


def cli_without_hooks(pid, checkout):
    """True when cargo process `pid` builds the engine's CLI for development without `test-hooks`:
    `cargo build` or `cargo run` of `rerail-engine` (by `--bin` or `-p`, or with no package named, from
    `native/`) on the dev or debugging profile, for this machine. Such a build compiles every
    dependency crate apart from the tests' build, and keeps a second set of incremental state
    (native/Cargo.toml, `test-hooks`)."""
    try:
        words = subprocess.run(["ps", "-o", "args=", "-p", str(pid)], capture_output=True, text=True,
                               timeout=5).stdout.split()
    except (OSError, subprocess.TimeoutExpired):
        return False
    for i, w in enumerate(words):
        if w in ("build", "run", "b", "r"):
            rest = words[i + 1:]
            break
    else:
        return False
    if "--" in rest:
        rest = rest[:rest.index("--")]
    if any("test-hooks" in r or r == "--all-features" or r == "--no-default-features" for r in rest):
        return False
    if any(r in ("--release", "-r") or r.startswith("--target") for r in rest):
        return False
    for j, r in enumerate(rest):
        prof = r.split("=", 1)[1] if r.startswith("--profile=") else (rest[j + 1] if r == "--profile" and j + 1 < len(rest) else None)
        if prof is not None and prof not in ("dev", "debugging"):
            return False
    named = [r.split("=", 1)[1] if "=" in r else (rest[j + 1] if j + 1 < len(rest) else "")
             for j, r in enumerate(rest) if r.split("=", 1)[0] in ("--bin", "-p", "--package")]
    if named:
        return "rerail-engine" in named
    if any(r in ("--lib", "--bins", "--examples", "--workspace", "--all-targets") for r in rest):
        return False
    try:
        out = subprocess.run(["lsof", "-a", "-p", str(pid), "-d", "cwd", "-Fn"], capture_output=True, text=True,
                             timeout=5).stdout
    except (OSError, subprocess.TimeoutExpired):
        return False
    cwd = next((l[1:] for l in out.splitlines() if l.startswith("n")), "")
    return bool(checkout) and os.path.realpath(cwd) == os.path.realpath(os.path.join(checkout, "native"))


def whole_tier(pid):
    """The command line of cargo process `pid` when it runs a whole test tier here (`cargo test` or
    `cargo nextest run` with no filter, or nextest's gate profile), else None."""
    def ps(pid):
        try:
            out = subprocess.run(["ps", "-o", "ppid=,args=", "-p", str(pid)], capture_output=True, text=True,
                                 timeout=5).stdout.split()
        except (OSError, subprocess.TimeoutExpired):
            return None, []
        return (out[0], out[1:]) if out else (None, [])
    parent, words = ps(pid)
    # nextest builds through `cargo test --no-run`: its own command line, above, has the filter
    if "--no-run" in words and parent:
        _, words = ps(parent)
        if "nextest" not in words:
            return None
    for i, w in enumerate(words):
        if w == "test" or (w == "run" and "nextest" in words[:i]):
            rest = words[i + 1:]
            break
    else:
        return None
    if "nextest" in words and any(r == "gate" or r == "--profile=gate" for r in rest):
        return " ".join(words)
    j = 0
    while j < len(rest):
        r = rest[j]
        if r == "--" or r in ("-E", "--filterset", "--filter-expr") or r.startswith(("-E=", "--filterset=")):
            return None  # a filter (after `--`, or an expression)
        if r in VALUED:
            j += 2
            continue
        if not r.startswith("-"):
            return None  # a test name filter
        j += 1
    return " ".join(words)


def advise(checkout, rustc):
    """At the first compile of each cargo command (once per cargo process): what this build does
    that it should not, on stderr, where the agent running it reads it. Its own nice value, a rustc
    other than the checkout pins, and the checkout's latest `diagnose --brief` findings (the cleaning
    job writes them, at most hourly). RERAIL_FARM_QUIET=1 keeps it quiet."""
    if os.environ.get("RERAIL_FARM_QUIET"):
        return
    cargo = os.getppid()
    said = state("said", str(cargo))
    try:
        if time.time() - os.stat(said).st_mtime < ADVICE_SECONDS:
            return
    except OSError:
        pass
    try:
        open(said, "w").close()
        for name in os.listdir(os.path.dirname(said)):
            if name.isdigit():
                try:
                    os.kill(int(name), 0)
                except ProcessLookupError:
                    os.unlink(os.path.join(os.path.dirname(said), name))
                except OSError:
                    pass
    except OSError:
        return
    lines = []
    nice = os.getpriority(os.PRIO_PROCESS, 0)
    if nice < 19:
        lines.append(f"this build runs at nice {nice}: run every build and test at `nice -n 19` (AGENTS.md)")
    m = re.search(r"/toolchains/(\d+\.\d+\.\d+)-", rustc)
    try:
        pinned = re.search(r'^\s*channel\s*=\s*"(\d+\.\d+\.\d+)"', open(os.path.join(checkout, "rust-toolchain.toml")).read(), re.M)
    except OSError:
        pinned = None
    if m and pinned and m.group(1) != pinned.group(1):
        lines.append(f"this build runs rustc {m.group(1)}, but the checkout pins {pinned.group(1)}: its crates are cached "
                     "apart from every other worktree's. Use rustup's cargo (`which cargo` is ~/.cargo/bin/cargo), "
                     "without `cargo +VERSION` or RUSTUP_TOOLCHAIN")
    tier = whole_tier(cargo)
    if tier and not os.environ.get("RERAIL_FARM_TESTS_HERE"):
        free = [h for h in usable("tests") if spare(h) >= 4]
        if free:
            lines.append(f"this command runs a whole test tier here while {free[0]['name']} has "
                         f"{spare(free[0]):.0f} cores to spare: `scripts/dev/test-remote -- {tier.split(' ', 1)[-1]}` "
                         "runs it there (RERAIL_FARM_TESTS_HERE=1 says it must run here)")
    if cli_without_hooks(cargo, checkout):
        lines.append("this builds the engine's CLI without `--features test-hooks`, so every dependency crate is compiled "
                     "apart from the tests' build of it, and keeps a second set of incremental state: "
                     "`nice -n 19 cargo build --bin rerail-engine --features test-hooks` (AGENTS.md)")
    lease = leased(HERE)
    if lease:
        lines.append(f"a benchmark lease holds this machine until {time.strftime('%H:%M:%S', time.localtime(lease['until']))} "
                     f"at the latest ({lease.get('label') or 'unnamed'}): this build's compiles wait for it, and anything heavy "
                     "you run here now (tests, Relax) is unfair to the benchmark: run it with scripts/dev/test-remote, or after")
    lines += interference_advice(checkout)
    try:
        f = findings_path(checkout)
        if time.time() - os.stat(f).st_mtime < 2 * DIAGNOSE_SECONDS:
            lines += [l.rstrip() for l in open(f) if l.strip()]
    except OSError:
        pass
    if lines:
        out = ["warning: rerail-farm: this checkout's builds miss the shared cache or the machine's rules:"]
        out += [f"warning: rerail-farm:   {l}" for l in lines[:10]]
        out.append("warning: rerail-farm: details: ~/.local/share/rerail-farm/diagnose (RERAIL_FARM_QUIET=1 silences this)")
        sys.stderr.write("\n".join(out) + "\n")
        sys.stderr.flush()


def wrapper(rustc, args):
    """cargo's RUSTC_WRAPPER."""
    env = dict(os.environ)
    crate = value(args, "--crate-name")
    if crate is None or not os.path.exists(SCCACHE):
        os.execv(rustc, [rustc] + args)
    out_dir = value(args, "--out-dir")
    checkout = checkout_of(out_dir or os.getcwd()) or checkout_of(os.getcwd())
    if not is_rerail(checkout):
        # Not Rerail's (another project, or a registry crate built outside any checkout): plain
        # sccache, as before the farm.
        plain = os.path.join(HOME, ".cargo/bin/sccache")
        if os.path.exists(plain):
            os.execv(plain, [plain, rustc] + args)
        os.execv(rustc, [rustc] + args)
    # Not cargo's probes (`--print`, crate `___`): cargo keeps their output, a failure included, in
    # `target/.rustc_info.json`, and would replay a refusal after its cause was gone.
    if crate != "___" and not any(a == "--print" or a.startswith("--print=") for a in args):
        canonical(crate)
    join(checkout)
    target = target_of(out_dir)
    if target and crate != "___":
        tidy(target, checkout)
        advise(checkout, rustc)
    # Every compile of a checkout's waits while this machine is leased for a benchmark: a hot
    # incremental one too, which runs rustc itself (scripts/dev/quiet-run).
    if crate != "___":
        wait_for_lease(HERE, f"{crate} waits to compile")
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


# What every Rerail build on this machine has in its environment (`setup` writes it into
# ~/.cargo/config.toml, forced): a build that differs would rerun build scripts and key its crates
# apart from every other worktree's (README.md, "One configuration").
CANONICAL = {"CC": os.path.join(SHARE, "farm-cc"), "CXX": os.path.join(SHARE, "farm-c++"), "SOURCE_DATE_EPOCH": "0"}
# What none has: each changes what a build script builds, so a build with it rebuilds them and every
# crate above them, and its outputs are cached apart. (JEMALLOC_OVERRIDE links another jemalloc
# than the one Cargo.lock names, which a version bump would not notice.)
FORBIDDEN = ("JEMALLOC_OVERRIDE", "AARCH64_APPLE_DARWIN_JEMALLOC_OVERRIDE")


def canonical(crate):
    """Refuses a Rerail compile whose environment is not the canonical one (CANONICAL, FORBIDDEN): it
    would not reuse what every other worktree compiled. `RERAIL_FARM_ANY_ENV=1` allows it, for an
    experiment that must differ."""
    if os.environ.get("RERAIL_FARM_ANY_ENV") == "1":
        return
    wrong = [f"{k}={os.environ.get(k)!r} (should be {v!r})" for k, v in CANONICAL.items() if os.environ.get(k) != v]
    wrong += [f"{k} is set" for k in FORBIDDEN if k in os.environ]
    if wrong:
        sys.stderr.write(
            f"error: rerail-farm: {crate}: this build's environment is not the machine's canonical one, so it would not "
            f"reuse what the other worktrees compiled: {'; '.join(wrong)}. The values come from ~/.cargo/config.toml "
            "(scripts/build/farm/setup writes it); do not export them in a script. RERAIL_FARM_ANY_ENV=1 allows a build "
            "that must differ (scripts/build/farm/README.md, \"One configuration\").\n")
        sys.exit(1)


def cc(argv0, args):
    """`farm-cc` and `farm-c++`, the C and C++ compilers of build scripts (`CC`, `CXX`): clang through
    the farm's sccache, so a build script's C (SQLite, tree-sitter's grammars, simdjson) is compiled
    once for every target directory and worktree, not once per target directory."""
    compiler = "/usr/bin/clang++" if os.path.basename(argv0).endswith("++") else "/usr/bin/clang"
    if not os.path.exists(SCCACHE):
        os.execv(compiler, [compiler] + args)
    checkout = checkout_of(os.environ.get("OUT_DIR") or os.getcwd()) or checkout_of(os.getcwd())
    if not is_rerail(checkout):
        os.execv(compiler, [compiler] + args)
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
PROBE_BYTES = 32 << 20
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
              " sysctl -n vm.loadavg; mount -t nfs | sed 's/.* on \\(.*\\) (.*/\\1/'")
    try:
        out = subprocess.run(ssh(name, script, "health"), capture_output=True, text=True, timeout=10)
    except subprocess.TimeoutExpired:
        out = None
    if out is None or out.returncode != 0 or len(out.stdout.splitlines()) < 5:
        if route(name) and not fell_back:
            # The address that reached it over its link is gone (it was renumbered, or the link
            # is down): back to what its name resolves to.
            set_route(name, None)
            return probe(host, cfg, fell_back=True, recovered=recovered)
        blocked = local_network_blocked(name)
        h["why"] = blocked or "unreachable (off, asleep, or off the network)"
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
    try:
        h["load"] = float(lines[4].strip("{} ").split()[0])
    except (ValueError, IndexError):
        h["load"] = None
    h["mounts"] = [m for m in lines[5:] if m]
    # The round trip is the least of three: each starts an ssh process here, which a loaded Mac
    # delays by up to a few hundred ms, and that is not the link.
    rtts = []
    for _ in range(3):
        started = time.time()
        try:
            subprocess.run(ssh(name, "true", "health"), capture_output=True, timeout=5)
            rtts.append(round((time.time() - started) * 1000))
        except subprocess.TimeoutExpired:
            rtts.append(5000)
            break
    h["rtt_ms"] = min(rtts)
    link = link_of(host)
    if time.time() - h["mbps_at"] > PROBE_SECONDS or (h["mbps"] or 0) < cfg["min_mbps"]:
        data = os.urandom(PROBE_BYTES)  # made before the clock starts: 70 ms on a loaded Mac
        # A plain pipe: `run(input=, capture_output=True)` writes stdin 512 bytes at a time while
        # it reads, and measured itself (57 MB/s where the pipe does 270).
        proc = subprocess.Popen(ssh(name, "cat >/dev/null", "health"), stdin=subprocess.PIPE,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        timer = threading.Timer(10, proc.kill)
        timer.start()
        started = time.time()
        try:
            proc.stdin.write(data)
            proc.stdin.close()
        except OSError:
            pass
        killed = proc.wait() != 0 and not timer.is_alive()
        timer.cancel()
        h["mbps"] = 0 if killed else round(PROBE_BYTES / (1 << 20) / max(time.time() - started - h["rtt_ms"] / 1000, 0.01), 1)
        h["mbps_at"] = time.time()
    if link and arrival != link and not recovered and recover(host, arrival):
        return probe(host, cfg, fell_back=fell_back, recovered=True)
    h["route"] = route(name)
    if link and arrival != link:
        h["why"] = f"not on its link: reached over its {arrival or '?'}, not {link}"
    elif h["free_gb"] < cfg["min_free_gb"]:
        h["why"] = f"{h['free_gb']} GB free, under {cfg['min_free_gb']}"
    # The round trip and throughput stand in for the link when none is configured (a host that fell
    # back to WiFi is slow). With `link` confirmed they are only reported: an ssh command's round
    # trip measures how fast both machines schedule ssh, and a loaded build machine (a load of 70
    # and more) pushed it past the limits on the fast link, taking healthy hosts out mid-build.
    elif not link and h["rtt_ms"] > cfg["max_rtt_ms"]:
        h["why"] = f"a {h['rtt_ms']} ms round trip, over {cfg['max_rtt_ms']}"
    elif not link and (h["mbps"] or 0) < cfg["min_mbps"]:
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
        elif not h["ok"] and "Local Network privacy" in h["why"]:
            # Not the host: it is back the moment the permission is given, so no growing back-off.
            h["down_until"] = now + DOWN_SECONDS[0]
        elif not h["ok"]:
            h["failures"] = h.get("failures", 0) + 1
            h["down_until"] = now + min(DOWN_SECONDS[0] * 2 ** (h["failures"] - 1), DOWN_SECONDS[1])
        write_json(path, h)
        log(f"{name}: {'usable' if h['ok'] else 'not usable: ' + h['why']} ({h.get('rtt_ms')} ms, {h.get('mbps')} MB/s, {h.get('free_gb')} GB free)")
        return h
    finally:
        lock.release()


def spare(host):
    """The cores `host` has to spare now: its capacity (`capacity`) less its load average when last
    probed. Only an order between hosts (a test command goes to the one with most to spare), never
    a refusal: macOS's background work raises the load of an idle machine too."""
    return capacity(host) - (health(host).get("load") or 0)


def usable(kind=None):
    """The enabled hosts that are usable now, the most cores to spare first (`spare`: a host busy
    with its own work comes after an idle one with fewer cores); `kind` ("mirror", "tests") keeps
    those the configuration names for it."""
    out = []
    for host in hosts():
        if not host.get("enabled", True) or (kind and not host.get(kind, True)):
            continue
        if leased(host["name"]):
            continue
        if health(host).get("ok"):
            out.append(host)
    return sorted(out, key=lambda h: -spare(h))


def capacity(host):
    """How many compiles `host` takes now: none while it is not usable (`health`); one per core while
    nobody has used it for 5 minutes, half as many while someone does. Not its load average: macOS's
    background work (Photos' and Spotlight's analysis) keeps that high on an idle machine, and runs
    at a priority every compile preempts."""
    h = health(host)
    if not h.get("ok"):
        return 0
    return host["cores"] if not attended(host, h) else host["cores"] // 2


def attended(host, h=None):
    """Whether a person uses `host` now: input in the last USER_IDLE_SECONDS. Never for a host the
    configuration marks `unattended` (a machine nobody sits at, whose input device reports
    touches on its own)."""
    if host.get("unattended"):
        return False
    return (h or health(host)).get("idle_s", 0) <= USER_IDLE_SECONDS


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


def local_network_blocked(host):
    """Why `host` cannot be reached from this process when the network is not the cause, or None.
    macOS's Local Network privacy refuses an app's processes (and every program they start: ssh,
    nc) any connection to a local address, as "No route to host", while the host answers everyone
    else. So a connection that fails here so but succeeds when launchd starts it names the app."""
    g = dict(line.split(" ", 1) for line in subprocess.run(["ssh", "-G", host], capture_output=True, text=True)
             .stdout.splitlines() if " " in line)
    try:
        info = socket.getaddrinfo(g.get("hostname", host), 22, proto=socket.IPPROTO_TCP)
    except OSError:
        return None
    for family, _, _, _, sa in info[:4]:
        addr = f"{sa[0]}%{socket.if_indextoname(sa[3])}" if family == socket.AF_INET6 and sa[3] else sa[0]
        nc = ["/usr/bin/nc", "-v", "-z", "-G", "2", addr, str(g.get("port", 22))]
        try:
            here = subprocess.run(nc, capture_output=True, text=True, timeout=5)
        except subprocess.TimeoutExpired:
            continue
        if here.returncode == 0 or "No route to host" not in here.stderr:
            continue
        out = state("lnprobe", f"{os.getpid()}.txt")
        label = f"rerail-farm.lnprobe.{os.getpid()}"
        subprocess.run(["launchctl", "submit", "-l", label, "-o", out, "-e", out, "--"] + nc, capture_output=True)
        time.sleep(3)
        subprocess.run(["launchctl", "remove", label], capture_output=True)
        try:
            text = open(out).read()
            os.remove(out)
        except OSError:
            text = ""
        if "succeeded" in text:
            return (f"macOS's Local Network privacy blocks {responsible_app()} from local addresses (launchd reaches "
                    f"{host}): allow it in System Settings, Privacy & Security, Local Network")
    return None


def responsible_app():
    """The application this process runs under, for messages: the outermost .app among its
    ancestors (an interpreter's own bundle, Xcode's Python, is inside it)."""
    pid, found = os.getppid(), None
    for _ in range(40):
        out = subprocess.run(["ps", "-o", "ppid=,comm=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
        if not out:
            break
        ppid, comm = out.split(None, 1)
        m = re.search(r"/Applications/([^/]+)\.app/", comm)
        if m:
            found = m.group(1)
        pid = int(ppid)
        if pid <= 1:
            break
    return found or "this terminal's application"


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


# ---- leases: a quiet machine for a benchmark (scripts/dev/quiet-run, README.md "Quiet runs")
#
# A client asks for a machine (a host, or HERE) for a command. Requests queue per machine, fairly:
# the client that used the machine least in the last hour goes first. While a lease holds a
# machine the farm sends it nothing new (compile slots, test commands, local compiles here wait),
# A run has LEASE_SECONDS at most.
HERE = "here"
LEASE_SECONDS = 10 * 60
# The window fairness is judged over.
FAIR_SECONDS = 3600
# How long a warning to a checkout that loaded a leased machine is shown at its builds.
INTERFERED_SECONDS = 3 * 3600


def lease_dir(machine):
    return state("leases", machine, "x")[:-2]


def pid_alive(pid):
    try:
        os.kill(int(pid), 0)
        return True
    except (ValueError, TypeError, ProcessLookupError):
        return False
    except PermissionError:
        return True


def lease_of(machine):
    """The lease holding `machine` now, or None (one whose holder died or whose time is past goes)."""
    path = os.path.join(lease_dir(machine), "lease.json")
    lease = read_json(path, None)
    if lease is None:
        return None
    if not pid_alive(lease.get("pid")) or time.time() > lease.get("until", 0):
        try:
            os.unlink(path)
        except OSError:
            pass
        return None
    return lease


def leased(machine):
    """The lease that keeps the farm's work off `machine`, or None: not the caller's own (a lease
    holder's command runs with RERAIL_FARM_LEASE set to its id)."""
    lease = lease_of(machine)
    if lease is None or lease.get("id") == os.environ.get("RERAIL_FARM_LEASE"):
        return None
    return lease


def queue(machine):
    """The live requests for `machine`, in the order they are served: least use of the machine in
    the last FAIR_SECONDS first, then the earliest asked."""
    d = os.path.join(lease_dir(machine), "queue")
    os.makedirs(d, exist_ok=True)
    out = []
    for n in os.listdir(d):
        r = read_json(os.path.join(d, n), None)
        if r is None or not pid_alive(r.get("pid")):
            try:
                os.unlink(os.path.join(d, n))
            except OSError:
                pass
            continue
        out.append(r)
    used = {c: used_seconds(c, machine) for c in {r["client"] for r in out}}
    return sorted(out, key=lambda r: (used[r["client"]], r["asked"]))


def ledger():
    """Every lease that ended in the last FAIR_SECONDS (state/leases/ledger.jsonl)."""
    since = time.time() - FAIR_SECONDS
    out = []
    try:
        for line in open(state("leases", "ledger.jsonl")):
            try:
                e = json.loads(line)
            except ValueError:
                continue
            if e.get("ended", 0) > since:
                out.append(e)
    except OSError:
        pass
    return out


def used_seconds(client, machine=None):
    """The seconds `client` held leases in the last FAIR_SECONDS (on `machine`, or anywhere)."""
    since = time.time() - FAIR_SECONDS
    return sum(e["ended"] - max(e["started"], since) for e in ledger()
               if e["client"] == client and (machine is None or e["machine"] == machine))


def record_lease(lease, ended, how, report):
    with open(state("leases", "ledger.jsonl"), "a") as f:
        f.write(json.dumps({"machine": lease["machine"], "client": lease["client"], "label": lease.get("label"),
                            "started": lease["started"], "ended": ended, "how": how} | report) + "\n")


def interfered(checkout, entry):
    """Records that `checkout`'s processes loaded a leased machine: `advise` tells its builds."""
    path = state("interfered", key(checkout) + ".json")
    seen = [e for e in read_json(path, []) if time.time() - e.get("at", 0) < INTERFERED_SECONDS]
    write_json(path, seen + [entry])


def interference_advice(checkout):
    """The warnings `advise` prints for `checkout`: its processes loaded a leased machine lately."""
    out = []
    for e in read_json(state("interfered", key(checkout) + ".json"), []):
        if time.time() - e.get("at", 0) < INTERFERED_SECONDS:
            out.append(f"this checkout's {e['what']} used {e['cpu_s']:.0f} CPU-s on this machine during a benchmark lease "
                       f"({time.strftime('%H:%M', time.localtime(e['at']))}, {e.get('label') or 'unnamed'}): that is unfair to "
                       "the benchmark; check scripts/build/farm/status for a lease before heavy runs, or run them with "
                       "scripts/dev/test-remote")
    return out


def wait_for_lease(machine, why):
    """Waits while a lease holds `machine` (its time is bounded), saying so once: work held back is
    told why, so it knows it is waiting on a benchmark, not on the farm."""
    told = False
    while True:
        lease = leased(machine)
        if lease is None:
            return
        if not told:
            print(f"warning: rerail-farm: {why}: this machine is leased for a benchmark "
                  f"({lease.get('label') or 'unnamed'}) until {time.strftime('%H:%M:%S', time.localtime(lease['until']))} "
                  "at the latest (scripts/build/farm/status)", file=sys.stderr, flush=True)
            told = True
        time.sleep(1)


def remote_slot():
    """A free compile slot on a host, and its lock. Slots are taken in turn across the hosts (each
    one's first, then each one's second, ...), so every host works, not only the first listed."""
    if os.environ.get("RERAIL_FARM_LOCAL"):
        return None, None
    open_hosts = []
    for h in hosts():
        if not h.get("enabled", True):
            continue
        if leased(h["name"]):
            continue
        open_hosts.append((h, capacity(h)))
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


def tracked(checkout):
    out = subprocess.run(["git", "-C", checkout, "ls-files", "--cached", "--others", "--exclude-standard"],
                         capture_output=True, text=True)
    return [n for n in out.stdout.splitlines() if n]


def remote(host, rustc, args, env, since):
    """Compiles on `host`, through the live mirror (README.md). False when anything failed, the
    compile included: the caller compiles here then, and only that run's output is shown. Logs why,
    and where the time went."""
    name = host["name"]
    cwd = os.getcwd()
    out_dir = os.path.abspath(value(args, "--out-dir"))
    crate, extra = value(args, "--crate-name"), codegen(args, "extra-filename") or ""
    times, mark = [], [time.time()]

    def lap(label):
        now = time.time()
        times.append(f"{label} {now - mark[0]:.2f}")
        mark[0] = now

    def give_up(why):
        log(f"{crate}: not on {name}: {why} ({', '.join(times)})")
        return False

    leak = leak_marker(crate, extra, args)
    if os.path.exists(leak):
        return give_up("its outputs name the host's farm root (an earlier compile there)")
    return run_mirror(host, rustc, args, env, cwd, out_dir, crate, since, lap, give_up, leak)


def leak_marker(crate, extra, args):
    """Marks a crate whose remote outputs named the host's farm root (farm-agent's code 71): it
    compiles here from then on. Keyed on its arguments, so another version of it is tried again."""
    return state("leaks", key(crate + extra + "\0" + "\0".join(args)))


def run_mirror(host, rustc, args, env, cwd, out_dir, crate, since, lap, give_up, leak):
    """Compiles on a host through the live mirror (`"mirror": true`, README; mirror.py): the daemon
    here sends what changed of what the compile may read, then the compile, over one connection
    to the host; its outputs come back on it. What it read (the dep-info's files) must be what is
    here; else this compile runs here."""
    import mirror
    # rustc hashes each source file's real path into its output, remapped or not: a crate compiled
    # from another path is other bytes, and every crate above it would key apart from the same
    # crate compiled here. So only a crate whose sources the host has at the same path compiles
    # there: one of cargo's registry or git checkouts, whose files are the same on every machine,
    # read in place from the host's own copy (checked file for file first: farm-agent, `same`).
    # A crate whose build script generated sources (OUT_DIR) would read them from the farm's root.
    if not any(cwd.startswith(p + "/") for p in IN_PLACE):
        return give_up("its sources are not cargo's registry's (a path of this machine's only)")
    out = env.get("OUT_DIR")
    if out and os.path.isdir(out) and os.listdir(out):
        return give_up("its build script wrote OUT_DIR, which a compile there would read from another path")
    roots = [[out_dir, "deps"]]
    for d in values(args, "-L"):
        d = os.path.abspath(d.split("=", 1)[-1])
        if d != out_dir and d.startswith(HOME + "/") and os.path.isdir(d):
            roots.append([d, "top"])
    files = [os.path.abspath(e.split("=", 1)[1]) for e in values(args, "--extern") if "=" in e]
    extra = codegen(args, "extra-filename") or ""
    kinds = emits(args)
    outputs = [f"{crate}{extra}.d"] * ("dep-info" in kinds) + [f"lib{crate}{extra}.rmeta"] * ("metadata" in kinds) \
        + [f"lib{crate}{extra}.rlib"] * ("link" in kinds)
    run = {"cwd": cwd, "argv": [rustc] + args, "dirs": [out_dir], "inplace": [cwd],
           "outputs": [os.path.join(out_dir, o) for o in outputs],
           "depinfo": os.path.join(out_dir, f"{crate}{extra}.d") if "dep-info" in kinds else None,
           "env": {k: v for k, v in env.items() if k not in LOCAL_ONLY and not k.startswith(("SCCACHE_", "RERAIL_FARM_"))},
           # What the host keeps under its farm root, and maps there in the arguments and environment.
           "map": [r for r, _ in roots] + files}
    reply = mirror.compile({"host": host["name"], "since": since, "roots": roots, "files": files, "run": run})
    if reply is None:
        return give_up("the mirror daemon did not answer")
    t = reply.get("times", {})
    lap(f"sync {t.get('sync')} ({t.get('files')} files), compile {t.get('compile')}, mirror")
    if reply["code"] == 71:
        open(leak, "w").close()
    if reply["code"] != 0:
        return give_up(f"the compile failed ({reply['code']}): {reply.get('why', '')} {reply['stderr'][-400:]!r}")

    def here(path):
        try:
            st = os.stat(path)
            return [st.st_mtime_ns, st.st_size]
        except OSError:
            return None
    stale = [p for p, seen in reply["read"].items() if seen != "inplace" and seen != here(p)]
    if stale:
        return give_up(f"it read files that are not as they are here: {stale[:3]}")
    sys.stdout.buffer.write(reply["stdout"].encode("latin-1"))
    sys.stderr.buffer.write(reply["stderr"].encode("latin-1"))
    log(f"{crate}: compiled on {host['name']}, mirror ({t.get('sync')} s sync, {t.get('compile')} s compile, {len(reply['read'])} read)")
    return True


def farm_rustc(argv0, args):
    """The compiler sccache runs: rustc itself, here or on another Mac."""
    since = time.time()
    try:
        rustc = open(os.path.join(os.path.dirname(argv0), "real")).read().strip()
    except OSError:
        rustc = os.path.join(HOME, ".cargo/bin/rustc")
    # The farm's sccache server runs this, not cargo: cargo's jobserver descriptors are not open
    # here, and rustc would warn on every crate ("failed to connect to jobserver").
    for k in ("MAKEFLAGS", "MFLAGS", "CARGO_MAKEFLAGS"):
        os.environ.pop(k, None)
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
    wait_for_lease(HERE, f"{value(args, '--crate-name')} waits to compile")
    lock = local_slot()
    try:
        code = subprocess.run([rustc] + args).returncode
    finally:
        lock.release()
    sys.exit(code)
