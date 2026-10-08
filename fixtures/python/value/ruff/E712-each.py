#: E712
if res == True:
    pass
#: E712
if res != False:
    pass
#: E712
if True != res:
    pass
#: E712
if False == res:
    pass
#: E712
if res[1] == True:
    pass
#: E712
if res[1] != False:
    pass
#: E712
var = 1 if cond == True else -1 if cond == False else cond
#: E712
if (True) == TrueElement or x == TrueElement:
    pass

if res == True != False:
    pass

if(True) == TrueElement or x == TrueElement:
    pass

if (yield i) == True:
      print("even")

#: Okay
if x not in y:
    pass

if not (X in Y or X is Z):
    pass

if not (X in Y):
    pass

if x is not y:
    pass

if X is not Y is not Z:
    pass

if TrueElement.get_element(True) == TrueElement.get_element(False):
    pass

assert (not foo) in bar
assert {"x": not foo} in bar
assert [42, not foo] in bar

# https://github.com/astral-sh/ruff/issues/17582
if True == True: # No duplicated diagnostic
    pass

# Chains: one diagnostic per offending operator (M12b, finding.each).
if a == True == False:
    pass
if a != True != b == False:
    pass
if True == False:
    pass
if x == True == True:
    pass
if (x == True) == False:
    pass
if 1 == True != False:
    pass
if a == (True) != (False) == b:
    pass
