import asyncio
import itertools
import os
import time
from itertools import count as forever

import click


# ---- other loops that only end from inside -----------------------------------------------------------------------


def flag_loop(connect):
    done = False
    while not done:  # caught
        try:
            connect()
            done = True
        except OSError:
            time.sleep(1)


def counting_loop(connect):
    for _ in itertools.count():  # caught
        try:
            return connect()
        except OSError:
            time.sleep(1)


def aliased_counting_loop(connect):
    for _attempt in forever(1):  # caught
        try:
            return connect()
        except OSError:
            time.sleep(1)


def try_inside_if(connect, enabled):
    while True:  # caught
        if enabled:
            try:
                return connect()
            except OSError:
                time.sleep(1)


def try_inside_with(connect, lock):
    while True:  # caught
        with lock:
            try:
                conn = connect()
                break
            except OSError:
                time.sleep(1)
    return conn


def flag_bounded(connect):
    done = False
    attempts = 0
    while not done:
        try:
            connect()
            done = True
        except OSError:
            attempts += 1
            done = attempts > 5


def counting_bounded(connect):
    for attempt in itertools.count():
        if attempt > 5:
            raise TimeoutError("gave up")
        try:
            return connect()
        except OSError:
            time.sleep(1)


def try_in_a_nested_loop(connect, hosts):
    while True:
        for host in hosts:
            try:
                return connect(host)
            except OSError:
                time.sleep(1)


def flag_from_an_event(stop_event, poll):
    stopped = False
    while not stopped:
        try:
            poll()
        except OSError:
            time.sleep(1)
        stopped = stop_event.is_set()


# ---- a person's Ctrl-C is not a failure of the work --------------------------------------------------------------


def repl(loop):
    while True:
        try:
            loop.run_forever()
        except KeyboardInterrupt:
            print("KeyboardInterrupt (type exit() to leave)")
            continue
        else:
            break


async def console(session):
    while True:
        try:
            return await session.interact()
        except KeyboardInterrupt:
            await asyncio.sleep(0)


def repl_or_crash(loop):
    while True:  # caught
        try:
            loop.run_forever()
            break
        except (KeyboardInterrupt, OSError):
            continue


# ---- prompts reached through a helper ----------------------------------------------------------------------------


def _ask_credentials():
    email = input("Email: ")
    password = click.prompt("Password", hide_input=True)
    return email, password


def _create_admin(store):
    email, password = _ask_credentials()
    store.create_admin(email, password)


def _create_admin_from_env(store):
    email = os.getenv("ADMIN_EMAIL") or input("Email: ")
    store.create_admin(email, os.environ["ADMIN_PASSWORD"])


def _create_admin_if_interactive(store, interactive):
    if interactive:
        _ask_credentials()
    store.create_admin("admin", "admin")


def create_admin_interactive(store):
    while True:
        try:
            _create_admin(store)
            break
        except Exception as exc:
            print(f"Failed to create the admin user: {exc}")


def ask_until_valid(store):
    while True:
        try:
            return store.create_admin(*_ask_credentials())
        except ValueError as exc:
            print(exc)


def create_admin_from_env(store):
    while True:  # caught
        try:
            _create_admin_from_env(store)
            break
        except Exception as exc:
            print(f"Failed to create the admin user: {exc}")


def create_admin_maybe_interactive(store, interactive):
    while True:  # caught
        try:
            _create_admin_if_interactive(store, interactive)
            break
        except Exception as exc:
            print(f"Failed to create the admin user: {exc}")
