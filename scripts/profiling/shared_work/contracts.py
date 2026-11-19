"""Check explicit contracts and bindings. Names never prove SQL equivalence."""
from collections import defaultdict
import hashlib
import json
import re


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def digest(value):
    return hashlib.sha256(encoded(value).encode()).hexdigest()


def differences(left, right, path=""):
    """Keep JSON types distinct, including integer, boolean, and real values."""
    if type(left) is not type(right):
        return [{"field": path, "first": left, "second": right}]
    if isinstance(left, dict):
        rows = []
        for key in sorted(left.keys() | right.keys()):
            field = f"{path}.{key}" if path else key
            if key not in left or key not in right:
                rows.append({"field": field, "first": left.get(key), "second": right.get(key),
                             "missingFrom": "first" if key not in left else "second"})
            else:
                rows.extend(differences(left[key], right[key], field))
        return rows
    if isinstance(left, list) and len(left) == len(right):
        return [row for i, (a, b) in enumerate(zip(left, right))
                for row in differences(a, b, f"{path}[{i}]")]
    return [] if left == right else [{"field": path, "first": left, "second": right}]


def fields(value, required, optional=()):
    if not isinstance(value, dict):
        raise ValueError("expected an object")
    missing = set(required) - value.keys()
    unknown = value.keys() - set(required) - set(optional)
    if missing or unknown:
        raise ValueError(f"missing fields {sorted(missing)}; unknown fields {sorted(unknown)}")


def named_types(value):
    if not isinstance(value, dict) or any(not isinstance(k, str) or not isinstance(v, str)
                                          or not k or not v for k, v in value.items()):
        raise ValueError("inputs and options require named type strings")


def contract_definition(contract):
    fields(contract, {"name", "completion", "inputs", "options", "columns", "key",
                      "multiplicity", "ordering", "implementation"}, {"origin"})
    if not isinstance(contract["name"], str) or not re.fullmatch(
            r"[a-z][a-z0-9_]*(?:\.[a-z][a-z0-9_]*)+\.v[1-9][0-9]*", contract["name"]):
        raise ValueError("contract name requires a namespace and explicit .vN version")
    if contract["completion"] not in ("keys", "domain"):
        raise ValueError("completion must be keys or domain")
    if contract["multiplicity"] not in ("set", "bag"):
        raise ValueError("multiplicity must be set or bag")
    named_types(contract["inputs"])
    named_types(contract["options"])
    columns = contract["columns"]
    if not isinstance(columns, list) or not columns:
        raise ValueError("columns must be a nonempty array")
    names = []
    for column in columns:
        fields(column, {"name", "type", "nullable"})
        if not all(isinstance(column[k], str) and column[k] for k in ("name", "type")):
            raise ValueError("column names and types must be nonempty strings")
        if type(column["nullable"]) is not bool:
            raise ValueError("column nullable must be boolean")
        names.append(column["name"])
    if len(set(names)) != len(names):
        raise ValueError("duplicate output column")
    if not isinstance(contract["key"], list) or any(n not in names for n in contract["key"]):
        raise ValueError("key must contain declared output column names")
    if len(set(contract["key"])) != len(contract["key"]):
        raise ValueError("duplicate key column")
    if not isinstance(contract["ordering"], list):
        raise ValueError("ordering must be an array")
    for order in contract["ordering"]:
        fields(order, {"column", "direction", "nulls"})
        if order["column"] not in names or order["direction"] not in ("asc", "desc") or order["nulls"] not in ("first", "last"):
            raise ValueError("ordering requires a declared column, asc/desc direction, and first/last nulls")
    if contract["completion"] == "keys" and not contract["key"]:
        raise ValueError("keyed completion requires an input key in the output schema")
    if not isinstance(contract["implementation"], dict) or not contract["implementation"]:
        raise ValueError("implementation requires an explicit definition descriptor")
    return {k: v for k, v in contract.items() if k not in ("name", "origin")}


def typed_option(value, expected):
    fields(value, {"kind"}, {"value"})
    kind = value["kind"]
    if kind != expected:
        raise ValueError(f"option requires {expected}, received {kind}")
    data = value.get("value")
    kinds = {"integer": lambda v: type(v) is int,
             "real": lambda v: type(v) is float,
             "text": lambda v: isinstance(v, str),
             "boolean": lambda v: type(v) is bool,
             "blob": lambda v: isinstance(v, list) and all(type(b) is int and 0 <= b <= 255 for b in v),
             "null": lambda v: v is None}
    if kind not in kinds or not kinds[kind](data) or (kind != "null" and "value" not in value):
        raise ValueError(f"invalid {kind} option value")


def check(documents):
    """Documents are (source, parsed JSON) pairs. Nothing here executes rule code."""
    contracts, groups, signatures, identities, fingerprints = {}, defaultdict(list), {}, {}, {}
    clashes, errors, aliases, separate = [], [], [], []
    manifests = []
    for source, document in documents:
        try:
            fields(document, {"format", "snapshot", "contracts", "bindings"})
            if document["format"] != "rerail.shared-work-plan/v1":
                raise ValueError("unsupported plan format")
            if not isinstance(document["snapshot"], str) or not document["snapshot"]:
                raise ValueError("snapshot must be an explicit nonempty evaluation identity")
            if not all(isinstance(document[k], list) for k in ("contracts", "bindings")):
                raise ValueError("contracts and bindings must be arrays")
            encoded(document)
        except (ValueError, TypeError) as error:
            errors.append({"origin": source, "message": str(error)})
            continue
        manifests.append((source, document))
        for i, contract in enumerate(document["contracts"]):
            origin = {"file": source, "entry": f"contracts[{i}]",
                      "source": contract.get("origin") if isinstance(contract, dict) else None}
            try:
                definition = contract_definition(contract)
            except (ValueError, TypeError) as error:
                errors.append({"origin": origin, "message": str(error)})
                continue
            name = contract["name"]
            if name in contracts and differences(contracts[name][0], definition):
                clashes.append({"kind": "contract-name-clash", "name": name,
                                "firstOrigin": contracts[name][1], "secondOrigin": origin,
                                "differences": differences(contracts[name][0], definition)})
            else:
                contracts.setdefault(name, (definition, origin))
    for source, document in manifests:
        bindings = {}
        for i, binding in enumerate(document["bindings"]):
            origin = {"file": source, "entry": f"bindings[{i}]",
                      "source": binding.get("origin") if isinstance(binding, dict) else None}
            try:
                fields(binding, {"id", "contract", "inputs", "options", "consumers"}, {"origin", "domain", "demand"})
                if not isinstance(binding["id"], str) or not binding["id"]:
                    raise ValueError("binding id must be a nonempty string")
                if not isinstance(binding["contract"], str) or binding["contract"] not in contracts:
                    raise ValueError("unknown contract")
                definition = contracts[binding["contract"]][0]
                for key in ("inputs", "options"):
                    if not isinstance(binding[key], dict) or binding[key].keys() != definition[key].keys():
                        raise ValueError(f"{key} must match the contract's declared names")
                for name, expected in definition["options"].items():
                    typed_option(binding["options"][name], expected)
                if not isinstance(binding["consumers"], list) or not all(
                        isinstance(c, str) and c for c in binding["consumers"]):
                    raise ValueError("consumers must be nonempty name strings")
                if "demand" in binding and not isinstance(binding["demand"], list):
                    raise ValueError("demand must be an array of requested keys")
                if (definition["completion"] == "domain") != ("domain" in binding):
                    raise ValueError("domain is required exactly for domain completion")
                if "domain" in binding and binding["domain"] is None:
                    raise ValueError("domain must identify explicit sealed membership and versions")
                if binding["id"] in bindings:
                    previous, first_origin = bindings[binding["id"]]
                    clashes.append({"kind": "binding-name-clash", "name": binding["id"],
                                    "firstOrigin": first_origin, "secondOrigin": origin,
                                    "differences": differences(previous, binding)})
                    continue
                bindings[binding["id"]] = (binding, origin)
            except (ValueError, TypeError) as error:
                errors.append({"origin": origin, "message": str(error)})
        visiting, resolved = set(), {}

        def resolve(identifier):
            if identifier in resolved:
                return resolved[identifier]
            if identifier in visiting:
                raise ValueError(f"cyclic binding dependency at {identifier}")
            if identifier not in bindings:
                raise ValueError(f"missing input binding {identifier}")
            visiting.add(identifier)
            binding, _ = bindings[identifier]
            inputs = {}
            try:
                for slot, value in binding["inputs"].items():
                    if isinstance(value, str):
                        inputs[slot] = {"binding": resolve(value)}
                    else:
                        fields(value, {"external"})
                        external = value["external"]
                        if not isinstance(external, dict) or not all(
                                isinstance(external.get(k), str) and external[k] for k in ("name", "generation")):
                            raise ValueError("external inputs require a name and generation")
                        inputs[slot] = value
                signature = {"snapshot": document["snapshot"], "contract": binding["contract"],
                             "inputs": inputs, "options": binding["options"]}
                if "domain" in binding:
                    signature["domain"] = binding["domain"]
                full_key = encoded(signature)
                key = identities.setdefault(full_key, len(identities))
                signatures[key] = signature
                stable_inputs = {slot: {"binding": fingerprints[value["binding"]]}
                                 if "binding" in value else value for slot, value in inputs.items()}
                fingerprints[key] = digest({**signature, "inputs": stable_inputs})
                resolved[identifier] = key
                return key
            finally:
                visiting.remove(identifier)

        for identifier, (binding, origin) in bindings.items():
            try:
                identity = resolve(identifier)
            except (ValueError, TypeError) as error:
                errors.append({"origin": origin, "message": str(error)})
                continue
            groups[identity].append({"id": identifier, "origin": origin,
                                     "consumers": binding["consumers"],
                                     "requestedKeys": len(binding.get("demand", []))})
    by_definition, by_contract = defaultdict(list), defaultdict(list)
    for name, (definition, _) in contracts.items():
        by_definition[encoded(definition)].append(name)
    for names in by_definition.values():
        if len(names) > 1:
            aliases.append({"names": sorted(names), "reason": "identical declared contract definitions; review naming"})
    for identity in sorted(groups):
        by_contract[signatures[identity]["contract"]].append(identity)
    for name, identities in sorted(by_contract.items()):
        for identity in identities[1:]:
            first = identities[0]
            separate.append({"contract": name, "firstBinding": groups[first][0],
                             "secondBinding": groups[identity][0],
                             "differences": differences(signatures[first], signatures[identity])})
    valid = not errors and not clashes
    binding_groups = [{"group": key, "identitySha256": fingerprints[key],
                       "signature": signatures[key], "bindings": values}
                      for key, values in sorted(groups.items())]
    return {"format": "rerail.shared-work-check/v1", "valid": valid, "errors": errors,
            "clashes": clashes, "equivalentContractCandidates": aliases,
            "bindingGroups": binding_groups,
            "sharedBindings": [{"group": key, "identitySha256": fingerprints[key],
                                "contract": signatures[key]["contract"], "bindings": values}
                               for key, values in sorted(groups.items())
                               if valid and (len(values) > 1 or len({c for v in values for c in v["consumers"]}) > 1)],
            "separateBindings": separate,
            "limitations": ["Declared identity checks only. No SQL equivalence proof or runtime completion claim.",
                            "Hashes label diagnostic groups; the runtime registry must compare full definitions."]}
