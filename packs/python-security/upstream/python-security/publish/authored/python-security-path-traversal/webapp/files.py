import os

NOTES = "/srv/notes"


def read_note(name):
    with open(os.path.join(NOTES, name + ".md")) as fh:  # caught
        return fh.read()
