# Mirror of native/src/code_rule_dsl/compile/config.rs option substitution (defaults only).
import json,re
def case_key(v):
    if v is None: return "null"
    if isinstance(v,bool): return "true" if v else "false"
    if isinstance(v,str): return v
    return json.dumps(v)
def text(v):
    if isinstance(v,str): return v
    if isinstance(v,bool): return "true" if v else "false"
    if isinstance(v,(int,float)): return json.dumps(v)
    raise ValueError(v)
class Opts:
    def __init__(s,doc):
        s.v={}
        for c in doc["clauses"]:
            d=c.get("define")
            if d and "options" in d:
                for k,o in d["options"].items(): s.v[k]=o.get("default")
        for c in doc["clauses"]:
            d=c.get("define")
            if d and "config" in d:
                for k,o in d["config"].items(): s.v[k]=o
    def ref(s,val):
        if not(isinstance(val,dict) and "$option" in val): return None
        opt=s.v[val["$option"]]; spread=val.get("spread",False)
        if "cases" in val:
            k=case_key(opt)
            ch=val["cases"][k] if k in val["cases"] else val["default"]
            r=s.sub(ch)
        else:
            r=opt
            if val.get("escape")=="regex":
                r=[re.escape(text(i)) for i in r] if isinstance(r,list) else re.escape(text(r))
            if "map" in val: r=[val["map"].replace("{}",text(i)) for i in r]
            if "join" in val: r=val["join"].join(text(i) for i in r)
            if "template" in val: r=val["template"].replace("{}",text(r))
        return ("spread",r) if spread else ("value",r)
    def sub(s,val):
        r=s.ref(val)
        if r: return r[1]
        if isinstance(val,dict): return {k:s.sub(v) for k,v in val.items()}
        if isinstance(val,list):
            out=[]
            for i in val:
                r=s.ref(i)
                if r is None: out.append(s.sub(i))
                elif r[0]=="spread": out.extend(r[1])
                else: out.append(r[1])
            return out
        return val
def resolve(doc):
    o=Opts(doc)
    out=dict(doc)
    out["clauses"]=[]
    for c in doc["clauses"]:
        if "define" in c and "$option" not in c:
            d=dict(c); dd=dict(c["define"]); 
            for k in list(dd):
                if k not in("options","config"): dd[k]=o.sub(dd[k])
            d["define"]=dd; out["clauses"].append(d); continue
        r=o.ref(c)
        if r is None: out["clauses"].append(o.sub(c))
        elif r[0]=="spread": out["clauses"].extend(r[1])
        else: out["clauses"].append(r[1])
    for k in ("scope",):
        if k in out: out[k]=o.sub(out[k])
    return out
