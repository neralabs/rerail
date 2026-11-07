#!/usr/bin/env bash
#
# Builds the PHP the port readiness recorder runs into the prefix given (its `bin/php`): the release `.php-version`
# pins, from php.net's source checked against the sha256 php.net publishes, with every extension compiled in, so it
# runs with no php.ini (`php -n`, as the recorder runs every PHP process) and loads exactly the extensions
# `extensions.txt` lists. The record and nightly workflows build it with this script (.github/actions/php), and a
# maintainer records locally on the same build:
#
#   conformance/recorders/port-readiness/runtimes/php/build.sh "$PWD/.rerail/php/8.5.11"
#   PATH="$PWD/.rerail/php/8.5.11/bin:$PATH" npm run port-readiness:record -- --pack php --tool phpcs
#
# Linux needs the development packages the action installs (pkg-config, libxml2, sqlite3, oniguruma, OpenSSL, zlib,
# curl, libzip, ICU); macOS needs Xcode's command line tools and those libraries from Homebrew (FORMULAE below). The
# build fails unless its version is the pin and its extensions are extensions.txt's, so every machine records on the
# same profile.
set -euo pipefail

# A new PHP pin changes `.php-version` and these two lines together; the build fails until they agree.
PHP_VERSION=8.5.11
PHP_SHA256=d9be75c08e8c316f4c8f4194d8fbe1750a15f6a6d9d4e3fe72082abeeb800360
# Beyond PHP's defaults (ctype, dom, fileinfo, filter, iconv, libxml, pdo_sqlite, phar, posix, session, simplexml,
# sqlite3, tokenizer, xml, xmlreader, xmlwriter among them): mbstring for Psalm and Laravel, openssl, zlib, curl and
# zip for Composer, and intl, which Laravel uses when it is there.
CONFIGURE=(--disable-cgi --disable-phpdbg --without-pear --enable-mbstring --enable-intl --with-openssl --with-zlib --with-curl --with-zip)
# The Homebrew formulae a macOS build links against (Linux takes the same libraries from its distribution).
FORMULAE=(pkgconf libxml2 sqlite oniguruma openssl@3 zlib curl libzip icu4c)

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fail() { printf '%s\n' "$*" >&2; exit 1; }

[ "$#" -eq 1 ] || fail "usage: $0 <prefix>"
mkdir -p "$1"
prefix="$(cd "$1" && pwd)"
pinned="$(tr -d '[:space:]' < "$HERE/.php-version")"
[ "$pinned" = "$PHP_VERSION" ] || fail "runtimes/php/.php-version pins PHP $pinned and build.sh builds $PHP_VERSION: change build.sh's version and checksum to the pin's"

case "$(uname -s)" in
  Linux)
    jobs="$(nproc)"
    ;;
  Darwin)
    jobs="$(sysctl -n hw.ncpu)"
    missing=()
    for formula in "${FORMULAE[@]}"; do
      if ! formula_prefix="$(brew --prefix --installed "$formula" 2>/dev/null)"; then
        missing+=("$formula")
      elif [ -d "$formula_prefix/lib/pkgconfig" ]; then
        PKG_CONFIG_PATH="$formula_prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
      fi
    done
    [ "${#missing[@]}" -eq 0 ] || fail "the build links against Homebrew's libraries: brew install ${missing[*]}"
    export PKG_CONFIG_PATH
    # macOS's iconv is in its SDK, where configure does not look unless told.
    CONFIGURE+=("--with-iconv=$(xcrun --show-sdk-path)/usr")
    ;;
  *)
    fail "build.sh builds PHP on Linux and macOS, not $(uname -s)"
    ;;
esac

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL --retry 3 -o "$work/php.tar.xz" "https://www.php.net/distributions/php-$PHP_VERSION.tar.xz"
actual="$( (command -v sha256sum >/dev/null && sha256sum "$work/php.tar.xz" || shasum -a 256 "$work/php.tar.xz") | cut -d ' ' -f 1)"
[ "$actual" = "$PHP_SHA256" ] || fail "php-$PHP_VERSION.tar.xz is sha256 $actual; php.net publishes $PHP_SHA256"
mkdir "$work/source"
tar -xJf "$work/php.tar.xz" -C "$work/source" --strip-components=1
(
  cd "$work/source"
  ./configure --prefix="$prefix" "${CONFIGURE[@]}"
  make -j"$jobs"
  make install
)

# The recorder's own check (pinnedPhp in runtimes.ts), run here so a build that differs fails where it is made.
php="$prefix/bin/php"
version="$("$php" -n -r 'echo PHP_VERSION;')"
[ "$version" = "$pinned" ] || fail "$php is PHP $version; runtimes/php/.php-version pins $pinned"
if ! extensions="$(diff <("$php" -n -m | grep -v -e '^\[' -e '^$' | LC_ALL=C sort -u) "$HERE/extensions.txt")"; then
  fail "$php -n -m differs from runtimes/php/extensions.txt (< the build, > the pin):
$extensions"
fi
echo "$php is PHP $version with the extensions runtimes/php/extensions.txt lists"
