"""Conservative placement discovery in generated candidate SQL, without execution."""
from collections import Counter, defaultdict
import re

from .contracts import digest, encoded


TOKEN = re.compile(r"\s+|--[^\n]*|/\*[\s\S]*?\*/|'(?:''|[^'])*'|\"(?:\"\"|[^\"])*\"|"
                   r"`(?:``|[^`])*`|\[[^\]]*\]|\?[0-9]+|[A-Za-z_][A-Za-z_0-9]*|.")
IDENTIFIER = re.compile(r"[A-Za-z_][A-Za-z_0-9]*\Z")


def tokens(sql):
    return [(m.group(), m.start(), m.end()) for m in TOKEN.finditer(sql)
            if not m.group().isspace() and not m.group().startswith(("--", "/*"))]


def definitions(sql):
    """Parse one generated WITH scope. Nested scopes are deliberately unsupported."""
    lexemes = tokens(sql)
    closing, stack = {}, []
    for i, (token, _, _) in enumerate(lexemes):
        if token == "(":
            stack.append(i)
        elif token == ")":
            if not stack:
                raise ValueError("unbalanced SQL parentheses")
            closing[stack.pop()] = i
    if stack:
        raise ValueError("unbalanced SQL parentheses")
    depth, start = 0, None
    for i, (token, _, _) in enumerate(lexemes):
        if token.upper() == "WITH":
            if depth:
                raise ValueError("nested WITH scope requires compiler provenance")
            start = i
            break
        depth += (token == "(") - (token == ")")
    if start is None:
        return [], sql
    i = start + 1
    if lexemes[i][0].upper() == "RECURSIVE":
        i += 1
    found = []
    while i < len(lexemes):
        begin = i
        name = lexemes[i][0]
        if not IDENTIFIER.fullmatch(name) or lexemes[i + 1][0] != "(":
            raise ValueError("expected generated CTE with explicit columns")
        end_columns = closing[i + 1]
        columns = sql[lexemes[i + 1][2]:lexemes[end_columns][1]]
        i = end_columns + 1
        if lexemes[i][0].upper() != "AS":
            raise ValueError("expected CTE AS")
        i += 1
        materialized = False
        if lexemes[i][0].upper() == "NOT":
            i += 1
            if lexemes[i][0].upper() != "MATERIALIZED":
                raise ValueError("expected NOT MATERIALIZED")
            i += 1
        elif lexemes[i][0].upper() == "MATERIALIZED":
            materialized = True
            i += 1
        if lexemes[i][0] != "(":
            raise ValueError("expected CTE body")
        end = closing[i]
        if any(t.upper() == "WITH" for t, _, _ in lexemes[i + 1:end]):
            raise ValueError("nested WITH scope requires compiler provenance")
        body = sql[lexemes[i][2]:lexemes[end][1]]
        found.append((name, columns, body, materialized, sql[lexemes[begin][1]:lexemes[end][2]]))
        i = end + 1
        if i == len(lexemes) or lexemes[i][0] != ",":
            break
        i += 1
    if len({x[0] for x in found}) != len(found):
        raise ValueError("duplicate CTE name within one scope")
    return found, sql[lexemes[i][1]:] if i < len(lexemes) else ""


class Interner:
    def __init__(self):
        self.identities = {}
        self.fingerprints = {}

    def intern(self, definition):
        if definition not in self.identities:
            identity = len(self.identities) + 1
            self.identities[definition] = identity
            # IDs compare full definitions. Digests are stable diagnostic labels only.
            def stable(part):
                if isinstance(part, tuple):
                    if len(part) == 2 and part[0] == "reference":
                        return ("reference", self.fingerprints[part[1]])
                    return tuple(stable(p) for p in part)
                return part
            self.fingerprints[identity] = digest(stable(definition))
        return self.identities[definition]


def parts(body, environment, parameters, self_name=None):
    result, start = [], 0
    for token, begin, end in tokens(body):
        replacement = None
        name = token[1:-1].replace('""', '"') if token.startswith('"') else token
        if token.startswith("?") and token[1:].isdigit():
            index = int(token[1:]) - 1
            if not 0 <= index < len(parameters):
                raise ValueError(f"unbound parameter {token}")
            # The capture retains both SQLite type and value.
            replacement = ("parameter", encoded(parameters[index]))
        elif name == self_name:
            replacement = ("self",)
        elif name in environment:
            replacement = ("reference", environment[name])
        if replacement is not None:
            result.extend((("sql", body[start:begin]), replacement))
            start = end
    result.append(("sql", body[start:]))
    return tuple(result)


def canonical(sql, physical, parameters, interner):
    specs, tail = definitions(sql)
    environment, found = dict(physical), []
    future = {s[0] for s in specs}
    for name, columns, body, materialized, definition in specs:
        # Forward references need a real compiler graph, rather than textual guessing.
        future.remove(name)
        if any(token in future for token, _, _ in tokens(body)):
            raise ValueError("forward CTE reference requires compiler provenance")
        identity = interner.intern(("cte", columns, parts(body, environment, parameters, name)))
        environment[name] = identity
        found.append((name, materialized, definition, identity, columns))
    return environment, found, tail


def write_targets(sql):
    lexemes = [t for t, _, _ in tokens(sql)]
    for i, token in enumerate(lexemes):
        upper = token.upper()
        if upper == "INTO" and "INSERT" in (t.upper() for t in lexemes[max(0, i - 3):i]):
            yield lexemes[i + 1].strip('"')
        elif upper == "UPDATE":
            yield lexemes[i + 1].strip('"')
        elif upper == "DELETE" and lexemes[i + 1].upper() == "FROM":
            yield lexemes[i + 2].strip('"')


def discover(capture, source, limit=20):
    if capture.get("format") not in ("rerail.candidate-replay/v1", "rerail.candidate-replay/v2"):
        raise ValueError("expected candidate replay capture")
    hosts = [(f"stage:{i}:{s['name']}", s["sql"], s) for i, s in enumerate(capture["stages"])]
    hosts.extend((f"result:{i}", sql, None) for i, sql in enumerate(capture["queries"]))
    interner, physical, epochs = Interner(), {}, Counter()
    placements, samples, local_names, host_counts = defaultdict(list), {}, defaultdict(set), Counter()
    skipped = []
    for host, sql, stage in hosts:
        try:
            environment, found, tail = canonical(sql, physical, capture["parameters"], interner)
        except (ValueError, IndexError, KeyError) as error:
            skipped.append({"host": host, "reason": str(error)})
            found = []
            if stage and stage.get("temporary"):
                physical[stage["name"]] = interner.intern(("opaque-stage", host, sql))
        for name, materialized, definition, identity, columns in found:
            if materialized:
                placements[identity].append({"host": host, "localName": name})
                local_names[name].add(identity)
                samples[identity] = (definition, columns)
                host_counts[host] += 1
        if stage and stage.get("temporary") and found:
            # CTAS casts and projections are semantic inputs, not another alias for its CTE.
            physical[stage["name"]] = interner.intern(
                ("physical", stage.get("columns"), parts(tail, environment, capture["parameters"])))
        for target in write_targets(sql):
            epochs[target] += 1
            physical[target] = interner.intern(("mutable", target, epochs[target]))
    repeated = [identity for identity, places in placements.items()
                if len({p["host"] for p in places}) > 1]
    repeated.sort(key=lambda k: (-len({p["host"] for p in placements[k]}),
                                 interner.fingerprints[k]))
    candidates = []
    for identity in repeated[:limit]:
        definition, columns = samples[identity]
        constant = re.sub(r"\s+", "", definition).upper().endswith("(SELECT0,0WHERE0)")
        candidates.append({"definitionSha256": interner.fingerprints[identity],
                           "hostStatements": len({p["host"] for p in placements[identity]}),
                           "placements": len(placements[identity]), "columns": columns,
                           "witnesses": placements[identity][:8], "definition": definition[:1600],
                           "review": "constant-fold" if constant else "consider explicit producer contract"})
    return {"format": "rerail.shared-work-discovery/v1", "capture": source,
            "complete": not skipped, "hostStatements": len(hosts), "skippedHosts": skipped,
            "materializedPlacements": sum(map(len, placements.values())),
            "definitionsInMultipleHosts": len(repeated),
            "extraHostPlacements": sum(len({p['host'] for p in placements[k]}) - 1 for k in repeated),
            "largestHosts": host_counts.most_common(5), "candidates": candidates,
            "localNamesWithDifferentDefinitions": sum(len(v) > 1 for v in local_names.values()),
            "limitations": ["Static placements, not runtime builds, CPU savings, or safe automatic sharing.",
                            "Each capture is analyzed separately. File versions and mutable-write epochs remain distinct.",
                            "Only generated CTE structure is compared. Nested scopes and forward references are skipped explicitly.",
                            "Legacy captures identify SQL hosts, not rule consumers. Local name reuse across hosts is legal."]}
