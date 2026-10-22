c = 0
a = 1 or 2
b = 1 if c else True
s = True if c else False
open(a)
open(b)
open(s)
open(3, file="x.txt")
open("x.txt", "r", -1, None, None, None, False, closefd=True)


class Settings:
    fd = 3
    fd += 1
    if c:
        handle = 4
    with int("5") as wd:
        pass


open(Settings.fd)
open(Settings.handle)
open(Settings.wd)
