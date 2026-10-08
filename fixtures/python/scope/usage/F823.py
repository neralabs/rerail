import os

my_var = 0
my_dict = {}


def augments():
    my_var += 1


def reads_then_binds():
    print(my_var)
    print(my_var)
    my_var = 1


def declares_global():
    global my_var
    my_var += 1


def binds_twice():
    print(my_var)
    my_var = 1
    my_var = 2


def imports_later():
    print(os.sep)
    import os


def reads_in_a_comprehension():
    [my_var for _ in range(1)]
    my_var = 1


def reads_in_a_nested_function():
    def inner():
        return my_var

    my_var = 1
    return inner


def annotates_later():
    print(my_dict)
    my_dict: dict


def reads_in_order():
    value = my_var if my_dict else os
    my_dict = 1
    return value


class Holder:
    def method(self):
        print(my_var)
        my_var = 1


lambda_value = lambda: my_var
