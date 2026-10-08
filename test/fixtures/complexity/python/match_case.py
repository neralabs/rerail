"""Each case of a match counts once, the wildcard included; a case guard is an if and counts too."""


def match_at_limit(command):
    match command:
        case "start":
            return 1
        case "stop":
            return 2
        case "pause":
            return 3
        case "resume":
            return 4
        case _:
            return 0


def match_over_limit(command):
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
        case _:
            return 0


def guarded_match_over_limit(point):
    match point:
        case (x, 0) if x > 0:
            return "positive x axis"
        case (0, y) if y > 0:
            return "positive y axis"
        case (x, y) if x == y:
            return "diagonal"
    return "elsewhere"
