# Rerail's Claude Code status line. The registered status command reads this file into the shell Claude Code already
# started for it (`. <this file>`): it is never run as a program. It uses shell built-ins only (read, case, printf, [),
# so a refresh starts no process of its own; only a session's first draw may start `sleep` (below). POSIX sh: /bin/sh
# is bash 3.2 on macOS and dash on most of Linux.
#
# The daemon keeps each session's line in <Rerail home>/claude-status-lines/<session id>
# (src/presentation/claude-status-files.ts) and this prints it. The file holds the line three times, for a terminal
# with 24-bit colour, 256 colours and no colour; an empty file draws nothing.

# `set -- <node> <claude-status.js> [<settings>]` before this file: the person had a status line of their own, which
# Node runs before it adds Rerail's line (src/hosts/claude/claude-status.ts). The node set-up recorded while it is
# there, else `node` from PATH.
if [ "$#" -gt 0 ]; then
  if [ -x "$1" ]; then exec "$@"; fi
  shift
  exec node "$@"
fi

# Set while the person's own status line runs under Rerail's (claude-status.ts), to the pid that started it: a status
# line that runs Rerail's again draws nothing. A pid that is gone, or another user's now, does not count.
nested=
case ${_RERAIL_CLAUDE_STATUS_PARENT:-} in
  '' | *[!0-9]*) ;;
  *) if kill -0 "$_RERAIL_CLAUDE_STATUS_PARENT" 2>/dev/null; then nested=1; fi ;;
esac

# Sets `value` to what follows the first `"<$1>":` in `$line`, spaces skipped; empty when the line has none.
after_key() {
  value=
  rest=$line
  while :; do
    case $rest in *"\"$1\""*) ;; *) return ;; esac
    rest=${rest#*"\"$1\""}
    value=${rest#"${rest%%[! ]*}"}
    case $value in :*) ;; *) value=; continue ;; esac
    value=${value#:}
    value=${value#"${value%%[! ]*}"}
    return
  done
}

# From the JSON Claude Code writes to stdin: the session id, its first `"session_id": "<id>"`; and whether the session
# has had no reply yet, `"total_api_duration_ms": 0`. Every line is read, so Claude Code never writes to a closed pipe.
id=
unanswered=
while IFS= read -r line || [ -n "$line" ]; do
  if [ -z "$id" ]; then
    after_key session_id
    case $value in '"'*) value=${value#'"'}; id=${value%%'"'*} ;; esac
  fi
  after_key total_api_duration_ms
  case $value in 0 | 0[!0-9.]*) unanswered=1 ;; esac
done

# The id names a file, so one that could name anything else (`/`, `..`) is no session.
case $id in *[!A-Za-z0-9_-]*) id= ;; esac

# As `colourDepth` (src/presentation/rerail-mark.ts): NO_COLOR wins, COLORTERM alone says 24-bit, else 256 colours.
# The depth is also the line of the session's file drawn for it.
if [ -n "${NO_COLOR:-}" ]; then
  depth=3
  brand='R E R A I L'
  green='' red='' plain=''
else
  case ${COLORTERM:-} in
    [Tt][Rr][Uu][Ee][Cc][Oo][Ll][Oo][Rr] | 24[Bb][Ii][Tt])
      depth=1
      brand='\033[38;2;165;173;251mR\033[39m \033[38;2;179;182;246mE\033[39m \033[38;2;193;191;242mR\033[39m \033[38;2;208;200;237mA\033[39m \033[38;2;222;209;233mI\033[39m \033[38;2;236;218;228mL\033[39m'
      ;;
    *)
      depth=2
      brand='\033[38;5;147mR\033[39m \033[38;5;147mE\033[39m \033[38;5;147mR\033[39m \033[38;5;189mA\033[39m \033[38;5;253mI\033[39m \033[38;5;254mL\033[39m'
      ;;
  esac
  green='\033[32m' red='\033[31m' plain='\033[39m'
fi

# The Rerail home: RERAIL_HOME when it is an absolute path, else ~/.rerail. Set-up records an absolute one; the daemon
# would read another relative to its own folder, which this cannot know.
case ${RERAIL_HOME:-} in
  /*) home=$RERAIL_HOME ;;
  *) home=$HOME/.rerail ;;
esac
status=$home/claude-status-lines

# Claude Code draws the status line as a session starts, about a second before the session's start hook has told the
# daemon of it, and draws again only with the session's first reply. So until that reply, a draw that finds no file
# waits for one: 7 s at most, in six sleeps at most, the only program this script starts. The Node status line waits
# the same (STARTING_WAITS_MS in claude-status.ts). Without a `sleep` to start there is no wait.
if [ -z "$nested" ] && [ -n "$id" ] && [ -n "$unanswered" ] && [ ! -e "$status/$id" ] && command -v sleep >/dev/null 2>&1; then
  for pause in 0.2 0.3 0.5 1 2 3; do
    sleep "$pause" || break
    if [ -e "$status/$id" ]; then break; fi
  done
fi

if [ -n "$nested" ]; then
  :
elif [ -z "$id" ]; then
  printf "$brand $green●$plain session unavailable\n"
elif [ -r "$status/$id" ]; then
  count=0
  while IFS= read -r line || [ -n "$line" ]; do
    count=$((count + 1))
    if [ "$count" -eq "$depth" ]; then
      printf '%s\n' "$line"
      break
    fi
  done < "$status/$id"
elif [ -e "$home/daemon.json" ]; then
  # A daemon is recorded and keeps no line for this session: Rerail is not on in its repository, or the session's
  # start never reached the daemon. Either way there is nothing to draw.
  :
else
  printf "$brand $red✕$plain not running\n"
fi
