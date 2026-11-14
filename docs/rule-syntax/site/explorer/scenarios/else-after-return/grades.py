def grade(score):
    if score >= 90:
        return "A"
    else:
        return "B"


def band(score):
    if score >= 90:
        return "high"
    elif score >= 50:
        return "mid"
    return "low"


def flat(score):
    if score >= 90:
        return "A"
    return "B"
