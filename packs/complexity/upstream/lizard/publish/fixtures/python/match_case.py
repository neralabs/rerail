"""A match counts once however many cases it has, the wildcard included; a case guard is an if and counts too."""


def match_counts_once(command):
    match command:
        case "start":
            return 1
        case "stop":
            return 2
        case "pause":
            return 3
        case "resume":
            return 4
        case "reset":
            return 5
        case "skip":
            return 6
        case _:
            return 0


def match_over_limit(command, retries):
    if retries > 3:
        return -1
    match command:
        case "start":
            return 2 if retries else 1
        case "stop":
            return 3
    while retries > 0:
        retries -= 1
    if retries < 0 or not command:
        return -2
    return 4


def nested_matches(kind, mode):
    match kind:
        case "file":
            match mode:
                case "read":
                    return 1
                case "write":
                    return 2
            return 3
        case "directory":
            return 4
    return 0


def guarded_match_over_limit(point):
    match point:
        case (x, 0) if x > 0:
            return "positive x axis"
        case (0, y) if y > 0:
            return "positive y axis"
        case (x, y) if x == y:
            return "diagonal"
        case (x, y) if x == -y:
            return "anti-diagonal"
        case (x, _) if x < 0:
            return "left half"
    return "elsewhere"
