import time

caught_before = 1
try:
    pass
except Exception as caught_before:
    print(caught_before)
print(caught_before, time)

try:
    pass
except Exception as time:
    print(time)
print(time)

try:
    pass
except* Exception as grouped:
    print(grouped)
print(grouped)

try:
    pass
except Exception as only_here:
    print(only_here, lambda: only_here)
print(only_here)


def reads_later():
    return only_here, grouped, caught_before


def handles():
    try:
        pass
    except Exception as local_error:
        print(local_error)
    return local_error
