# Literal equality (Ruff's ComparableLiteral): implicit concatenations compare part by part.
def f1(a):
    if a == 1:
        return 1
    elif a == 0x1:
        return 2
    elif a == 2:
        return 3
def f2(a):
    if a == 1:
        return 1
    elif a == 1.0:
        return 2
    elif a == 2:
        return 3
def f3(a):
    if a == 1:
        return 1
    elif a == True:
        return 2
    elif a == 2:
        return 3
def f4(a):
    if a == "a":
        return 1
    elif a == 'a':
        return 2
    elif a == "b":
        return 3
def f5(a):
    if a == "ab":
        return 1
    elif a == "a" "b":
        return 2
    elif a == "\x61b":
        return 3
    elif a == "c":
        return 4
def f6(a):
    if a == 0j:
        return 1
    elif a == 0:
        return 2
    elif a == 0.0:
        return 3
def f7(a):
    if a == b"a":
        return 1
    elif a == "a":
        return 2
    elif a == None:
        return 3
def f8(a):
    if a == 1:
        return 1
    elif a == 2:
        return 2
    elif a == 1:
        return 3
    elif a == 2:
        return 4
def f9(a):
    if (a == 1):
        return 1
    elif a == (2):
        return 2
    elif ((a == (3))):
        return 3
def f10(a):
    if a == 1.5:
        return 1
    elif a == 1.50:
        return 2
    elif a == 15e-1:
        return 3
    elif a == 3:
        return 4
def f11(a):
    if a == "\N{DIGIT ONE}":
        return 1
    elif a == "1":
        return 2
    elif a == "2":
        return 3
def f12(a):
    if a == 1:
        return 1
    elif a == 2:
        return 2
    elif a == 3:
        return 3
    else:
        return 4
def f13(a):
    if a == 1:
        return 1
    elif a == 2:
        return 2
    else:
        if a == 1:
            return 1
        elif a == 1:
            return 2
        elif a == 3:
            return 3
def f14(a):
    if a == "a" "b":
        return 1
    elif a == 'a' 'b':
        return 2
    elif a == "c":
        return 3
def f15(a):
    if a == "a" "b":
        return 1
    elif a == "a" "b":
        return 2
    elif a == "c":
        return 3
def f16(a):
    if a == ("a" "b"):
        return 1
    elif a == "a" "b":
        return 2
    elif a == "c":
        return 3
def f17(a):
    if a == "a" "b":
        return 1
    elif a == "ab" "":
        return 2
    elif a == "c":
        return 3
