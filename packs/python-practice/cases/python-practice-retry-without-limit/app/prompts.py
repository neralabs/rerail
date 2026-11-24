import time

import click


# Not a retry: each turn asks the person at the terminal again.
def ask_name(validate):
    while True:
        name = input("Name: ")
        try:
            return validate(name)
        except ValueError as exc:
            print(exc)


def ask_publisher(validate):
    while True:
        publisher = click.prompt("Publisher", type=str)
        try:
            validate(publisher)
            break
        except ValueError as exc:
            click.secho(str(exc), fg="red")
    return publisher


# The prompt runs once, before the loop: the loop retries the same call.
def save_profile(api):
    name = input("Name: ")
    while True:  # caught
        try:
            return api.save(name)
        except ConnectionError:
            time.sleep(1)
