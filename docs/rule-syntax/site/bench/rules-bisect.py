# rules-bisect.py <rules dir> <corpus> <always.rr> <csv pattern>: the smallest set of other rule files (ddmin) that, with
# <always.rr>, still prints <pattern> in violation.csv. Each try in a fresh dir under b002-repro/bis/<n>.
import os, shutil, subprocess, sys
SP = os.path.dirname(os.path.abspath(__file__))
rules, corpus, always, pat = sys.argv[1:5]
others = sorted(f for f in os.listdir(rules) if f.endswith('.rr') and f != always)
tries = [0]
def run(sub):
    tries[0] += 1
    d = f"{SP}/b002-repro/bis/{tries[0]}"; os.makedirs(d + "/r")
    for f in sub + [always]: shutil.copy(os.path.join(rules, f), d + "/r/")
    env = dict(os.environ, RERAIL_SQL_STRICT="1", RERAIL_TAPE_CACHE=d + "/store.sqlite", RERAIL_RR_LIB=f"{SP}/rs-now/lib")
    subprocess.run(["nice", "-n", "19", f"{SP}/engine-cur", "logic", d + "/r", corpus, d + "/o"], env=env, capture_output=True, timeout=300)
    try: hit = pat in open(d + "/o/violation.csv").read()
    except FileNotFoundError: hit = False
    print(f"try {tries[0]}: {len(sub)} rules -> {hit}", flush=True)
    return hit
assert run(others), "not reproduced"
n = 2
while len(others) >= 2:
    size = max(len(others) // n, 1); done = False
    for i in range(0, len(others), size):
        rest = others[:i] + others[i+size:]
        if run(rest): others, n, done = rest, max(n - 1, 2), True; break
    if not done:
        if size == 1: break
        n = min(n * 2, len(others))
print("minimal:", others)
