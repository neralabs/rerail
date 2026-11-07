"""Check realgraph.py's copy of LEG-083 against the real linter's own output.

    python3 -I vs_cache.py REPO COMMIT GRAPH

The data layer's fact engine caches each file's facts under REPO/tmp/lint_fact_store, keyed by the
sha256 of the source. Where a cached file has the content the file has at COMMIT, its cached
`legacy_execution_surfaces` violations are what the real linter reported for it. This compares
their LEG-083 rows (line, function, eager function) with GRAPH's R answers, file by file. It reads
the cache as JSON; nothing from the repository runs.
"""
import hashlib, json, os, re, subprocess, sys
repo, commit, graph = sys.argv[1:4]
git=["git","-C",repo]
paths=[p for p in subprocess.run(git+["ls-tree","-r","--name-only",commit],capture_output=True,text=True,check=True).stdout.splitlines() if p.endswith(".py")]
b=subprocess.Popen(git+["cat-file","--batch"],stdin=subprocess.PIPE,stdout=subprocess.PIPE)
byhash={}
for p in paths:
    b.stdin.write(f"{commit}:{p}\n".encode()); b.stdin.flush()
    h=b.stdout.readline().split(); data=b.stdout.read(int(h[2])); b.stdout.read(1)
    try: byhash[hashlib.sha256(data.decode("utf-8").encode("utf-8")).hexdigest()]=p
    except UnicodeDecodeError: pass
# mine
cur=None; sym={}; fil={}; mine={}
for line in open(graph):
    w=line.rstrip("\n").split(" ")
    if w[0]=="F": cur=w[1]
    elif w[0]=="N": sym[int(w[1])]=" ".join(w[4:]); fil[int(w[1])]=cur
    elif w[0]=="R" and w[2]!="-":
        r=int(w[1]); mine.setdefault(fil[r],set()).add((int(w[2]),sym[r],sym[int(w[3])]))
store=os.path.join(repo,"tmp/lint_fact_store/by_source")
real={}; versions=set(); seen=set()
for f in os.listdir(store):
    d=json.load(open(os.path.join(store,f)))
    p=byhash.get(d.get("content_hash"))
    if not p: continue
    for k,v in d["facts"].items():
        if not k.startswith("legacy_execution_surfaces"): continue
        if v.get("error") is not None or not isinstance(v.get("value"),list): continue
        versions.add((k,v["cache_version"])); seen.add(p)
        for x in v["value"]:
            if x["code"]!="LEG-083": continue
            m=re.match(r"(.*) reaches eager lazy-terminal (\w+)\(\) in (.*) without", x["message"])
            real.setdefault(p,set()).add((x["line_number"],m.group(1),m.group(3)))
print("cache entries matching files at the commit:", len(seen), "of", len(paths)); print("versions:", versions)
agree=dis=0
for p in sorted(seen):
    a=mine.get(p,set()); r=real.get(p,set())
    if a==r: agree+=1
    else:
        dis+=1
        print("DIFF", p, "\n  mine only:", sorted(a-r)[:5], "\n  real only:", sorted(r-a)[:5])
print("files agree:", agree, "differ:", dis)
print("LEG-083 in matched files: mine", sum(len(mine.get(p,()))for p in seen), "real", sum(len(real.get(p,())) for p in seen))
unm=[p for p in mine if p not in seen]; print("my findings in files with no cache entry:", sum(len(mine[p]) for p in unm), unm[:8])
