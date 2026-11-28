#!/bin/sh
# One command for a whole session with the Dongle Max in hand (spec 033, decision D55): flash the
# radio image once, run every check of the roadmap's list that needs no hand gesture, print one
# report line per step and one verdict. It calls scripts/box.sh step after step (the budgets of
# decision D39) and the host program for the update key and the signature.
#
#   sh scripts/box-session.sh [--yes-flash] [--backup <file>] [--factory-reset] [--with-gestures]
#                             [--port <device>] [--address <address>[:<port>]]
#   sh scripts/box-session.sh --api-only <address>[:<port>] [--factory-reset]
#
# Without --yes-flash it flashes nothing and writes nothing through the API: the read-only steps
# only. `--yes-flash` is the user's own yes of decision D6, for this run: typed by the user in a
# terminal (it is refused when /dev/tty cannot be opened), never added by an agent. It needs
# --backup <file> (or SOLITE_BOX_BACKUP), the original firmware's backup, which box.sh checks.
# The flash always keeps `storage`. --factory-reset also runs the factory reset (it erases the
# home). --with-gestures asks the steps that need a person one by one, on the terminal.
# --api-only runs the API groups straight against an address (the Mac host's --serve): no box.
#
# The box's password: SOLITE_PASSWORD (user SOLITE_USER, admin when unset), else the password file
# ($SOLITE_PASSWORD_FILE, by default $HOME/.config/solite/box-password; <run folder>/password with
# --api-only); a box with no account is set up with it, or with a password generated into that
# file. Never an argument, never printed.
#
# The standard output is the report; every command's full output goes to <run folder>/session.log.
# Run it as: sh scripts/quiet.sh --tail 60 sh scripts/box-session.sh --yes-flash --backup <file>
# Exit: 0 no step KO, 1 one is, 2 a usage error (nothing run).
#
# Environment: SOLITE_SESSION_ROOT (the run folders, $TMPDIR/solite-box-session), SOLITE_BOX_SCRIPT
# (box.sh), SOLITE_HOST_BIN (the host program), SOLITE_SESSION_BUILDER (the firmware builds),
# SOLITE_RADIO_TARGET_DIR (the radio build folder), SOLITE_PYTHON (python3 for /ws, busy, reader),
# SOLITE_HOME_LATITUDE, SOLITE_HOME_LONGITUDE, SOLITE_HOME_TIMEZONE (the home's location, never in
# the repository), SOLITE_SESSION_{CLOCK,BOOT,CONFIRM,ROLLBACK,RATE}_WAIT and SOLITE_SESSION_POLL
# (seconds).
#
# `--group auth|home|update|reset|permit|script` is how the script calls itself inside `box.sh saves` (and
# how the tests call one group): it runs one group against SOLITE_BOX_ADDRESS, with the run folder
# in SOLITE_SESSION_DIR. `permit` only closes the pairing window (before measure-api.sh, B6). The
# groups read SOLITE_SESSION_API_ONLY (set by --api-only: the target is the Mac host, so the
# box-only checks are skipped) and SOLITE_SESSION_RATE_WAIT (seconds to wait after the rate-limit
# check: --api-only sets 61; in a box run the next `saves` restarts the box).
set -u

SELF=$(cd "$(dirname "$0")" && pwd -P)/$(basename "$0")
ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
CALLER_DIR=$(pwd -P)
umask 077

# ---------------------------------------------------------------------------------------------
# Shared by the outer run and the groups
# ---------------------------------------------------------------------------------------------

JSON='Content-Type: application/json'
OCTET='Content-Type: application/octet-stream'
SOLITE_USER=${SOLITE_USER:-admin}
PYTHON=${SOLITE_PYTHON:-python3}

# The value at a path of a JSON file ($1 the file, $2 the path with / between the keys); true,
# false and null as words; fails when there is none.
jget() {
  python3 -c '
import sys, json
try:
    v = json.load(open(sys.argv[1]))
    for k in sys.argv[2].split("/"):
        v = v[k] if isinstance(v, dict) else v[int(k)]
except Exception:
    sys.exit(1)
print("true" if v is True else "false" if v is False else "null" if v is None else v)' "$1" "$2"
}

# ---------------------------------------------------------------------------------------------
# The groups: one request or a short fixed sequence each, one line per check
# ---------------------------------------------------------------------------------------------

# A check's line: $1 its number, $2 its name, $3 OK|KO|skipped, $4 the detail or the reason.
check() {
  if [ -n "${4:-}" ]; then
    printf 'box-session: %s.%s %s: %s (%s)\n' "$GID" "$1" "$2" "$3" "$4"
  else
    printf 'box-session: %s.%s %s: %s\n' "$GID" "$1" "$2" "$3"
  fi
  [ "$3" != KO ] || GKO=1
}

# `http <method> <path> [curl arguments]`: the answer's body in $TMP/body, its status in CODE and
# its time in TIME (000 when nothing answered).
http() {
  _m=$1
  _p=$2
  shift 2
  : >"$TMP/body"
  CODE=$(curl -sS -o "$TMP/body" -w '%{http_code} %{time_total}' --max-time "${HTTP_MAX:-30}" -X "$_m" "$API$_p" "$@" 2>/dev/null) || CODE="000 0"
  TIME=${CODE#* }
  CODE=${CODE%% *}
  printf '%s %s -> %s\n' "$_m" "$_p" "$CODE" >>"$SDIR/session.log" 2>/dev/null
}

# The `error` text of the last answer, else the status.
errtext() {
  _t=$(jget "$TMP/body" error 2>/dev/null) || _t=
  printf '%s' "${_t:-status $CODE}"
}

# Keeps the tokens of the last answer in the run folder (the serial log is searched for them) and
# makes the header file the requests use.
take_tokens() {
  TOKEN=$(jget "$TMP/body" accessToken) || TOKEN=
  RTOKEN=$(jget "$TMP/body" refreshToken) || RTOKEN=
  printf 'Authorization: Bearer %s\n' "$TOKEN" >"$AUTH_FILE"
  [ -z "$TOKEN" ] || printf '%s\n' "$TOKEN" >>"$SDIR/tokens"
  [ -z "$RTOKEN" ] || printf '%s\n' "$RTOKEN" >>"$SDIR/tokens"
}

# The password: SOLITE_PASSWORD, else the password file's. Sets PW (empty when there is none).
pick_password() {
  PW=${SOLITE_PASSWORD:-}
  if [ -z "$PW" ] && [ -f "$PWFILE" ]; then
    PW=$(head -n 1 "$PWFILE")
  fi
  return 0
}

# The body of a setup or a login, from the password on the standard input of printf (a built-in:
# the password is in no process list).
auth_body() {
  printf '%s' "$2" | python3 -c '
import sys, json
body = {"username": sys.argv[1], "password": sys.stdin.read()}
if sys.argv[2] == "setup":
    body["displayName"] = sys.argv[1]
print(json.dumps(body))' "$SOLITE_USER" "$1" >"$TMP/request"
}

# Setup (`$1` setup) or login (`$1` login) with the password `$2`; counts the attempt the rate
# limit counts; on 200 or 201 keeps the tokens.
attempt() {
  auth_body "$1" "$2"
  http POST "/auth/$1" -H "$JSON" --data-binary "@$TMP/request"
  rm -f "$TMP/request"
  [ "$CODE" = 429 ] || ATTEMPTS=$((ATTEMPTS + 1))
  if [ "$CODE" = 200 ] || [ "$CODE" = 201 ]; then take_tokens; fi
}

# A fresh login with the password; returns 0 when it answered 200.
relogin() {
  http GET /auth/status --max-time 5
  [ "$CODE" = 200 ] || return 1
  pick_password
  [ -n "$PW" ] || return 1
  attempt login "$PW"
  [ "$CODE" = 200 ]
}

# The preamble of the groups that need a token: a login, then the pairing closed (the reset that
# opens each `saves` is a power-on boot, which opens a pairing window: a device joining during a
# check would write devices.json over the budget).
group_login() {
  pick_password
  if [ -z "$PW" ]; then
    check 0 login KO "set SOLITE_PASSWORD (the box has an account)"
    return 1
  fi
  attempt login "$PW"
  if [ "$CODE" != 200 ]; then
    check 0 login KO "login refused ($CODE): $(errtext)"
    return 1
  fi
  close_pairing
  return 0
}

close_pairing() {
  http POST /integrations/solite/permit-join -H "@$AUTH_FILE" -H "$JSON" -d '{"time":0}'
  [ "$CODE" = 202 ] || check 0 "pairing closed" KO "permit-join answered $CODE: $(errtext)"
  return 0
}

# A WebSocket client with no dependency: $1 the address, $2 the token (empty: none offered).
# Prints `message` (a frame with data arrived) or `closed <code>`.
WS_PY=$(cat <<'PY'
import socket, sys, base64, os, struct
addr, token = sys.argv[1], sys.argv[2]
host, _, port = addr.partition(":")
try:
    s = socket.create_connection((host, int(port or 80)), timeout=10)
    key = base64.b64encode(os.urandom(16)).decode()
    head = "GET /ws HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n" % (addr, key)
    if token:
        head += "Sec-WebSocket-Protocol: bearer.none, bearer.%s\r\n" % token
    s.sendall((head + "\r\n").encode())
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = s.recv(4096)
        if not chunk:
            print("error no handshake answer")
            sys.exit(0)
        data += chunk
    status, _, rest = data.partition(b"\r\n\r\n")
    if b" 101 " not in status.split(b"\r\n")[0] + b" ":
        print("error handshake " + status.split(b"\r\n")[0].decode("latin1"))
        sys.exit(0)
    def take(n):
        global rest
        while len(rest) < n:
            chunk = s.recv(4096)
            if not chunk:
                raise OSError("closed")
            rest += chunk
        out, rest = rest[:n], rest[n:]
        return out
    b0, b1 = take(2)
    length = b1 & 0x7F
    if length == 126:
        length = struct.unpack(">H", take(2))[0]
    elif length == 127:
        length = struct.unpack(">Q", take(8))[0]
    payload = take(length)
    if b0 & 0x0F == 8:
        print("closed %d" % struct.unpack(">H", payload[:2])[0] if len(payload) >= 2 else "closed")
    else:
        print("message")
except Exception as error:
    print("error %s" % error)
PY
)

# Four requests whose body never comes, then a fifth: $1 the address. Prints `<the fifth's status
# line> | <how many of the four ended with 408>`.
BUSY_PY=$(cat <<'PY'
import socket, sys, time
addr = sys.argv[1]
host, _, port = addr.partition(":")
port = int(port or 80)
def status(s):
    data = b""
    try:
        while b"\r\n" not in data:
            chunk = s.recv(4096)
            if not chunk:
                break
            data += chunk
    except OSError:
        pass
    return data.split(b"\r\n")[0].decode("latin1")
held = []
for _ in range(4):
    s = socket.create_connection((host, port), timeout=10)
    s.sendall(("POST /api/v1/auth/login HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\nContent-Length: 200\r\n\r\n" % addr).encode())
    held.append(s)
time.sleep(1)
f = socket.create_connection((host, port), timeout=10)
f.sendall(("GET /api/v1/auth/status HTTP/1.1\r\nHost: %s\r\nConnection: close\r\n\r\n" % addr).encode())
fifth = status(f)
f.close()
deadline = time.time() + 10
ended = 0
for s in held:
    s.settimeout(max(0.1, deadline - time.time()))
    if " 408 " in status(s) + " ":
        ended += 1
print("%s | %d" % (fifth, ended))
PY
)

# A client that asks for a file and reads nothing: $1 the address, $2 the path, $3 seconds held.
READER_PY=$(cat <<'PY'
import socket, sys, time
addr = sys.argv[1]
host, _, port = addr.partition(":")
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1024)
s.settimeout(10)
s.connect((host, int(port or 80)))
s.sendall(("GET /%s HTTP/1.1\r\nHost: %s\r\n\r\n" % (sys.argv[2], addr)).encode())
time.sleep(float(sys.argv[3]))
PY
)

# The devices' dates (spec 028): $1 the devices' JSON file. Prints `OK <count>` or `KO <reason>`.
DATES_PY=$(cat <<'PY'
import sys, json, time, calendar
devices = json.load(open(sys.argv[1]))
now = time.time()
def parse(text):
    return calendar.timegm(time.strptime(text.rstrip("Z").split(".")[0].split("+")[0], "%Y-%m-%dT%H:%M:%S"))
count = 0
for d in devices:
    name = d.get("name", "?")
    for key in ("lastSeen", "createdAt", "updatedAt"):
        value = d.get(key)
        if value is None:
            continue
        t = parse(value)
        if t < 366 * 86400:
            print("KO %s of %s is in 1970 (%s)" % (key, name, value))
            sys.exit(0)
        if t > now + 60:
            print("KO %s of %s is in the future (%s)" % (key, name, value))
            sys.exit(0)
    seen = d.get("lastSeen")
    if seen is not None:
        age = now - parse(seen)
        limit = 600 if d.get("powerSource") == "mains" else 25 * 3600
        status = d.get("status")
        if age < limit - 60 and status != "online":
            print("KO %s was heard %d s ago and is %s, not online" % (name, age, status))
            sys.exit(0)
        if age > limit + 60 and status != "offline":
            print("KO %s was heard %d s ago and is %s, not offline" % (name, age, status))
            sys.exit(0)
    count += 1
print("OK %d" % count)
PY
)

# The root zone of the aggregation ($1 the file): `<sunrise> <sunset> <isDaylight>`, null as a word.
AGG_PY=$(cat <<'PY'
import sys, json
d = json.load(open(sys.argv[1]))
zone = {}
if isinstance(d, dict) and d:
    zone = d.get("00000000-0000-0000-0000-000000000001") or d[next(iter(d))]
def word(key):
    v = zone.get(key)
    return "null" if v is None else str(v).lower() if isinstance(v, bool) else str(v)
print(word("sunrise"), word("sunset"), word("isDaylight"))
PY
)

# 1970 in a device's dates ($1 the devices' file): prints `1970` when one is.
EPOCH_PY=$(cat <<'PY'
import sys, json, time, calendar
for d in json.load(open(sys.argv[1])):
    for key in ("lastSeen", "createdAt", "updatedAt"):
        v = d.get(key)
        if v and calendar.timegm(time.strptime(v.rstrip("Z").split(".")[0], "%Y-%m-%dT%H:%M:%S")) < 366 * 86400:
            print("1970")
            sys.exit(0)
PY
)

# 24 hexadecimal characters from /dev/urandom.
generate_password() {
  od -An -tx1 -N12 /dev/urandom | tr -d ' \n'
}

# The name of check $2 of group $1, for the checks that are not run.
check_name() {
  case "$1:$2" in
    auth:2) echo account ;; auth:3) echo "login time" ;; auth:4) echo gate ;; auth:5) echo websocket ;;
    auth:6) echo "pairing behind the token" ;; auth:7) echo "rate limit" ;;
    update:3) echo install ;; update:4) echo "restart on the new version" ;;
    update:5) echo "second upload" ;; update:6) echo rollback ;;
    *) echo "check $2" ;;
  esac
}

# ---- auth (J4, spec 029) ----

group_auth() {
  ATTEMPTS=0
  http GET /auth/status
  SETUP=
  if [ "$CODE" = 200 ]; then SETUP=$(jget "$TMP/body" setupRequired) || SETUP=; fi
  if [ "$CODE" = 200 ] && [ -n "$SETUP" ]; then
    check 1 status OK "setupRequired $SETUP"
  else
    check 1 status KO "GET /auth/status answered $CODE"
    for n in 2 3 4 5 6 7; do check "$n" "$(check_name auth "$n")" skipped "after check 1"; done
    return
  fi

  # 2. The account.
  pick_password
  if [ "$SETUP" = true ]; then
    created=
    if [ -z "$PW" ]; then
      PW=$(generate_password)
      mkdir -p -m 700 "$(dirname "$PWFILE")" 2>/dev/null
      if (set -C; printf '%s\n' "$PW" >"$PWFILE") 2>/dev/null; then
        chmod 600 "$PWFILE" 2>/dev/null
        created=$PWFILE
        printf '%s\n' "$PWFILE" >"$SDIR/password-created"
      else
        PW=
      fi
    fi
    if [ -z "$PW" ]; then
      check 2 account KO "no password: set SOLITE_PASSWORD"
    else
      attempt setup "$PW"
      if [ "$CODE" = 201 ]; then
        : >"$SDIR/account-usable"
        if [ -n "$created" ]; then check 2 account OK "account set up; password in $created"; else check 2 account OK "account set up"; fi
      else
        check 2 account KO "setup answered $CODE: $(errtext)"
      fi
    fi
  elif [ -z "$PW" ]; then
    check 2 account KO "set SOLITE_PASSWORD (the box has an account)"
  else
    attempt login "$PW"
    if [ "$CODE" = 200 ]; then
      : >"$SDIR/account-usable"
      check 2 account OK "logged in"
    elif [ "$CODE" = 401 ]; then
      check 2 account KO "the password does not open the box's account"
    else
      check 2 account KO "login answered $CODE: $(errtext)"
    fi
  fi

  # The pairing window closes as soon as there is a token (a device joining would write devices.json).
  [ -z "$TOKEN" ] || close_pairing

  # 3. One login, timed.
  if [ -z "$PW" ]; then
    check 3 "login time" skipped "no password"
  else
    attempt login "$PW"
    if [ "$CODE" != 200 ]; then
      check 3 "login time" KO "login answered $CODE: $(errtext)"
    elif awk -v t="$TIME" 'BEGIN { exit !(t > 2) }'; then
      check 3 "login time" KO "login in $TIME s, over 2 s"
    else
      check 3 "login time" OK "login in $TIME s"
    fi
  fi

  # 4. The gate.
  http GET /devices
  if [ "$CODE" = 401 ]; then check 4 gate OK; else check 4 gate KO "GET /devices without a token answered $CODE"; fi

  # 5. /ws.
  if ! command -v "$PYTHON" >/dev/null 2>&1; then
    check 5 websocket skipped "no python3"
  else
    plain=$($PYTHON -c "$WS_PY" "$ADDR" "")
    if [ -z "$TOKEN" ]; then
      check 5 websocket skipped "no token (closed: $plain)"
    else
      with=$($PYTHON -c "$WS_PY" "$ADDR" "$TOKEN")
      if [ "$plain" = "closed 4001" ] && [ "$with" = message ]; then
        check 5 websocket OK "$plain without a token, a message with it"
      else
        check 5 websocket KO "without a token: $plain; with it: $with"
      fi
    fi
  fi

  # 6. Pairing behind the token (closed since check 2, see close_pairing there).
  http POST /integrations/solite/permit-join -H "$JSON" -d '{"time":0}'
  bare=$CODE
  if [ -z "$TOKEN" ]; then
    check 6 "pairing behind the token" skipped "no token (without one: $bare)"
  else
    http POST /integrations/solite/permit-join -H "@$AUTH_FILE" -H "$JSON" -d '{"time":0}'
    open=$(jget "$TMP/body" permitJoin) || open=
    if [ "$bare" = 401 ] && [ "$CODE" = 202 ] && [ "$open" = false ]; then
      check 6 "pairing behind the token" OK "401 without a token, 202 and closed with it"
    else
      check 6 "pairing behind the token" KO "without a token $bare; with it $CODE, permitJoin $open"
    fi
  fi

  # 7. The rate limit, last: wrong passwords until the first 429.
  found=
  i=1
  while [ "$i" -le 12 ]; do
    attempt login "not-the-password-$i"
    if [ "$CODE" = 429 ]; then found=$((ATTEMPTS + 1)); break; fi
    i=$((i + 1))
  done
  if [ -z "$found" ]; then
    check 7 "rate limit" KO "no 429 within 12 attempts"
  elif [ "$found" = 11 ]; then
    check 7 "rate limit" OK "429 at attempt $found"
  else
    check 7 "rate limit" KO "429 at attempt $found, the 11th of the minute expected"
  fi
  # The next `saves` restarts the box, which clears the limit; against the Mac host (--api-only)
  # the outer run sets the wait (61 s), and the tests set it when their host is not restarted.
  if [ -n "${SOLITE_SESSION_RATE_WAIT:-}" ]; then
    sleep "$SOLITE_SESSION_RATE_WAIT"
  fi
}

# ---- home (specs 026, 027, 028, 023) ----

group_home() {
  group_login || return
  HOME_SET=false
  [ -n "${SOLITE_HOME_LATITUDE:-}" ] && [ -n "${SOLITE_HOME_LONGITUDE:-}" ] && [ -n "${SOLITE_HOME_TIMEZONE:-}" ] && HOME_SET=true

  # 1. The full backup (spec 035): the export (a key derivation estimated at 11 to 14 s on the
  # box, 70 to 88 s measured once before D59, hence a timeout above the export's own 120 s limit),
  # then its restore with a wrong password, which must change nothing.
  HTTP_MAX=130
  http GET /backup -H "@$AUTH_FILE" -H "X-Backup-Password: box-session-test"
  if [ "$CODE" != 200 ]; then
    check 1 backup KO "GET /backup answered $CODE: $(errtext)"
  elif [ "$(head -c 8 "$TMP/body")" != SOLITEBK ]; then
    check 1 backup KO "the file does not start with SOLITEBK"
  else
    cp "$TMP/body" "$TMP/backup.slb"
    http POST /backup -H "@$AUTH_FILE" -H "Content-Type: application/octet-stream" \
      -H "X-Backup-Password: not-the-test-password" --data-binary "@$TMP/backup.slb"
    if [ "$CODE" = 400 ] && [ "$(errtext)" = "Wrong backup password" ]; then
      check 1 backup OK "exported, wrong-password restore refused 400"
    elif [ "$CODE" = 403 ]; then
      # A restore needs the user's presence (10 min after a power-on) before the password is read.
      check 1 backup skipped "exported; restore not checked (presence: $(errtext))"
    elif [ "$CODE" = 409 ]; then
      # The pairing window (180 s after a power-on) refuses a restore before the password is read.
      check 1 backup skipped "exported; restore not checked (pairing window open: $(errtext))"
    else
      check 1 backup KO "POST /backup with a wrong password answered $CODE: $(errtext)"
    fi
  fi
  HTTP_MAX=30

  # 2. The location and the time zone.
  if $HOME_SET; then
    python3 -c '
import sys, json
print(json.dumps({"home.latitude": sys.argv[1], "home.longitude": sys.argv[2], "home.timezone": sys.argv[3]}))' \
      "$SOLITE_HOME_LATITUDE" "$SOLITE_HOME_LONGITUDE" "$SOLITE_HOME_TIMEZONE" >"$TMP/settings.json"
    http PUT /settings -H "@$AUTH_FILE" -H "$JSON" --data-binary "@$TMP/settings.json"
    put=$CODE
    http GET /settings -H "@$AUTH_FILE"
    if [ "$put" = 200 ] && [ "$CODE" = 200 ] \
      && [ "$(jget "$TMP/body" home.latitude 2>/dev/null)" = "$SOLITE_HOME_LATITUDE" ] \
      && [ "$(jget "$TMP/body" home.longitude 2>/dev/null)" = "$SOLITE_HOME_LONGITUDE" ] \
      && [ "$(jget "$TMP/body" home.timezone 2>/dev/null)" = "$SOLITE_HOME_TIMEZONE" ]; then
      printf '%s\n%s\n%s\n' "$SOLITE_HOME_LATITUDE" "$SOLITE_HOME_LONGITUDE" "$SOLITE_HOME_TIMEZONE" >"$SDIR/location"
      check 2 "location and time zone" OK "set and read back"
    else
      check 2 "location and time zone" KO "PUT answered $put, the settings read back with $CODE do not hold the values"
    fi
  else
    http GET /settings -H "@$AUTH_FILE"
    if [ "$CODE" != 200 ]; then
      check 2 "location and time zone" KO "GET /settings answered $CODE"
    else
      cp "$TMP/body" "$TMP/settings.json"
      http PUT /settings -H "@$AUTH_FILE" -H "$JSON" --data-binary "@$TMP/settings.json"
      if [ "$CODE" = 200 ]; then check 2 "location and time zone" OK "settings put back"; else check 2 "location and time zone" KO "PUT answered $CODE: $(errtext)"; fi
    fi
  fi
  http GET /settings -H "@$AUTH_FILE"
  HAS_LOCATION=false
  if [ "$CODE" = 200 ]; then
    [ -n "$(jget "$TMP/body" home.latitude 2>/dev/null)" ] && [ -n "$(jget "$TMP/body" home.longitude 2>/dev/null)" ] && HAS_LOCATION=true
    TZ_SET=$(jget "$TMP/body" home.timezone 2>/dev/null) || TZ_SET=
  else
    TZ_SET=
  fi

  # 3. The sun times: first the clock (the box serves before its clock is set).
  wait_clock=${SOLITE_SESSION_CLOCK_WAIT:-30}
  step=2
  [ "$wait_clock" -ge 2 ] || step=$wait_clock
  waited=0
  CLOCK_OK=false
  SUN=
  while :; do
    if $HAS_LOCATION; then
      http GET /zones/aggregation -H "@$AUTH_FILE"
      if [ "$CODE" = 200 ]; then
        SUN=$(python3 -c "$AGG_PY" "$TMP/body" 2>/dev/null) || SUN=
        case "$SUN" in "" | null\ *) ;; *) CLOCK_OK=true ;; esac
      fi
    else
      http GET /devices -H "@$AUTH_FILE"
      if [ "$CODE" = 200 ] && [ -z "$(python3 -c "$EPOCH_PY" "$TMP/body" 2>/dev/null)" ]; then CLOCK_OK=true; fi
    fi
    $CLOCK_OK && break
    [ "$waited" -lt "$wait_clock" ] || break
    sleep "$step"
    waited=$((waited + step))
  done
  if ! $CLOCK_OK; then
    check 3 "sun times" KO "clock not set within $wait_clock s"
  elif ! $HAS_LOCATION; then
    check 3 "sun times" skipped "set SOLITE_HOME_LATITUDE, SOLITE_HOME_LONGITUDE and SOLITE_HOME_TIMEZONE"
  else
    set -- $SUN
    case "$1:$2:$3" in
      *null*) check 3 "sun times" KO "the root zone holds sunrise $1, sunset $2, isDaylight $3" ;;
      *) check 3 "sun times" OK "sunrise $1, sunset $2, isDaylight $3" ;;
    esac
  fi

  # 4. The time zone.
  http GET /system/timezone -H "@$AUTH_FILE"
  tz=$(jget "$TMP/body" tz 2>/dev/null) || tz=
  source=$(jget "$TMP/body" source 2>/dev/null) || source=
  if [ "$CODE" != 200 ]; then
    check 4 "time zone" KO "GET /system/timezone answered $CODE"
  elif [ -n "$TZ_SET" ]; then
    if [ "$tz" = "$TZ_SET" ] && [ "$source" = env ]; then check 4 "time zone" OK "$tz ($source)"; else check 4 "time zone" KO "tz $tz, source $source; $TZ_SET and env expected"; fi
  else
    if [ "$tz" = UTC ] && [ "$source" = fallback ]; then check 4 "time zone" OK "UTC (fallback)"; else check 4 "time zone" KO "tz $tz, source $source; UTC and fallback expected"; fi
  fi

  # 5. The devices' dates.
  if ! $CLOCK_OK; then
    check 5 dates skipped "clock not set"
  else
    http GET /devices -H "@$AUTH_FILE"
    if [ "$CODE" != 200 ]; then
      check 5 dates KO "GET /devices answered $CODE"
    else
      verdict=$(python3 -c "$DATES_PY" "$TMP/body" 2>&1)
      case "$verdict" in
        "OK 0") check 5 dates skipped "no device" ;;
        OK*) check 5 dates OK "${verdict#OK } devices checked" ;;
        KO*) check 5 dates KO "${verdict#KO }" ;;
        *) check 5 dates KO "cannot read the devices: $verdict" ;;
      esac
    fi
  fi

  # 6 and 7. The server's limits (spec 023), on the box only.
  if [ -n "${SOLITE_SESSION_API_ONLY:-}" ]; then
    check 6 busy skipped "box only"
    check 7 "client that stops reading" skipped "box only"
  elif ! command -v "$PYTHON" >/dev/null 2>&1; then
    check 6 busy skipped "no python3"
    check 7 "client that stops reading" skipped "no python3"
  else
    answer=$($PYTHON -c "$BUSY_PY" "$ADDR" 2>&1)
    fifth=${answer%% | *}
    ended=${answer##* | }
    case "$fifth" in
      *" 503 "*) if [ "$ended" = 4 ]; then check 6 busy OK "the fifth request answered 503, the four ended with 408"; else check 6 busy KO "the fifth answered 503 but only $ended of the four ended with 408"; fi ;;
      *) check 6 busy KO "the fifth request got: ${fifth:-no answer}" ;;
    esac
    curl -sS -o "$TMP/body" --max-time 10 "http://$ADDR/" 2>/dev/null || : >"$TMP/body"
    script=$(grep -ao 'assets/index-[A-Za-z0-9_-]*\.js' "$TMP/body" | head -n 1)
    if [ -z "$script" ]; then
      check 7 "client that stops reading" skipped "no interface script in the home page"
    else
      $PYTHON -c "$READER_PY" "$ADDR" "$script" 6 >/dev/null 2>&1 &
      reader=$!
      sleep 1
      http GET /auth/status --max-time 5
      status=$CODE
      took=$TIME
      kill "$reader" 2>/dev/null
      wait "$reader" 2>/dev/null
      if [ "$status" = 200 ]; then check 7 "client that stops reading" OK "the API answered 200 in $took s"; else check 7 "client that stops reading" KO "the API answered $status while a client read nothing"; fi
    fi
  fi
}

# ---- update (J5, spec 032) ----

group_update() {
  next=$(cat "$SDIR/next.version" 2>/dev/null)
  boot_wait=${SOLITE_SESSION_BOOT_WAIT:-180}
  confirm_wait=${SOLITE_SESSION_CONFIRM_WAIT:-75}
  rollback_wait=${SOLITE_SESSION_ROLLBACK_WAIT:-300}
  poll=${SOLITE_SESSION_POLL:-5}
  group_login || return
  # 1. No token.
  http POST /system/update -H "$OCTET" -H 'Expect:' --data-binary @/dev/null
  if [ "$CODE" = 401 ]; then check 1 "update without a token" OK; else check 1 "update without a token" KO "answered $CODE, 401 expected"; fi
  # 2. A bad signature, on the header alone.
  NOKEY=false
  if [ ! -f "$SDIR/bad.sup" ] || [ -z "$next" ]; then
    check 2 "bad signature" KO "no signed file in the run folder (P4 did not run)"
    for n in 3 4 5 6; do check "$n" "$(check_name update "$n")" skipped "after check 2"; done
    return
  fi
  head -c 128 "$SDIR/bad.sup" >"$TMP/header"
  http POST /system/update -H "@$AUTH_FILE" -H "$OCTET" -H 'Expect:' --data-binary "@$TMP/header"
  if [ "$CODE" = 503 ]; then
    NOKEY=true
    check 2 "bad signature" skipped "this build has no update key"
  elif [ "$CODE" = 400 ] && [ "$(jget "$TMP/body" error 2>/dev/null)" = "Invalid signature" ]; then
    check 2 "bad signature" OK "400 Invalid signature"
  else
    check 2 "bad signature" KO "answered $CODE: $(errtext); 400 Invalid signature expected"
  fi
  if $NOKEY; then
    for n in 3 4 5 6; do check "$n" "$(check_name update "$n")" skipped "this build has no update key"; done
    return
  fi
  if [ -n "${SOLITE_SESSION_API_ONLY:-}" ]; then
    for n in 3 4 5 6; do check "$n" "$(check_name update "$n")" skipped "box only: the host does not restart"; done
    return
  fi
  # 3. The next version is installed.
  HTTP_MAX=600
  http POST /system/update -H "@$AUTH_FILE" -H "$OCTET" -H 'Expect:' --data-binary "@$SDIR/next.sup"
  HTTP_MAX=30
  if [ "$CODE" != 200 ]; then
    check 3 install KO "answered $CODE: $(errtext)"
    for n in 4 5 6; do check "$n" "$(check_name update "$n")" skipped "after check 3"; done
    return
  fi
  if ! grep -q "Installed $next" "$TMP/body"; then
    check 3 install KO "answered 200 without the message \"Installed $next, restarting\""
    for n in 4 5 6; do check "$n" "$(check_name update "$n")" skipped "after check 3"; done
    return
  fi
  check 3 install OK "installed $next"
  # 4. The box comes back with it, and keeps the settings.
  start=$(date +%s)
  back=false
  while :; do
    get_version
    if [ "$CODE" = 401 ] || [ "$CODE" = 403 ]; then
      relogin && get_version
    fi
    if [ "$CODE" = 200 ] && [ "$CURRENT" = "$next" ]; then back=true; break; fi
    [ $(($(date +%s) - start)) -lt "$boot_wait" ] || break
    sleep "$poll"
  done
  if ! $back; then
    check 4 "restart on the new version" KO "the box did not come back with $next within $boot_wait s"
    for n in 5 6; do check "$n" "$(check_name update "$n")" skipped "after check 4"; done
    return
  fi
  kept=
  if [ -f "$SDIR/location" ]; then
    http GET /settings -H "@$AUTH_FILE"
    lat=$(sed -n 1p "$SDIR/location")
    lon=$(sed -n 2p "$SDIR/location")
    zone=$(sed -n 3p "$SDIR/location")
    if [ "$CODE" = 200 ] && [ "$(jget "$TMP/body" home.latitude 2>/dev/null)" = "$lat" ] \
      && [ "$(jget "$TMP/body" home.longitude 2>/dev/null)" = "$lon" ] \
      && [ "$(jget "$TMP/body" home.timezone 2>/dev/null)" = "$zone" ]; then
      kept="; the settings survived the restart"
    else
      check 4 "restart on the new version" KO "the box came back with $next but the settings lost the location"
      kept=KO
    fi
  fi
  [ "$kept" = KO ] || check 4 "restart on the new version" OK "$next$kept"
  # 5. Confirmed: a second upload is accepted once the probation is over.
  sleep "$confirm_wait"
  http GET /system/version -H "@$AUTH_FILE" --max-time 10
  if [ "$CODE" = 401 ]; then relogin; fi
  UPTOKEN=$TOKEN
  HTTP_MAX=600
  http POST /system/update -H "@$AUTH_FILE" -H "$OCTET" -H 'Expect:' --data-binary "@$SDIR/rollback.sup"
  HTTP_MAX=30
  if [ "$CODE" != 200 ]; then
    check 5 "second upload" KO "answered $CODE: $(errtext)"
    check 6 rollback skipped "after check 5"
    return
  fi
  check 5 "second upload" OK "the rollback image was accepted"
  # 6. The rollback: a restart is proven, then the next version answers again.
  restarted=false
  rolled=false
  other=
  start=$(date +%s)
  while :; do
    printf 'Authorization: Bearer %s\n' "$UPTOKEN" >"$AUTH_FILE"
    http GET /system/version -H "@$AUTH_FILE" --max-time 10
    if [ "$CODE" = 401 ] && [ "$(jget "$TMP/body" error 2>/dev/null)" = "Invalid or expired token" ]; then
      restarted=true
      if relogin; then
        UPTOKEN=$TOKEN
        get_version
      fi
    elif [ "$CODE" = 200 ]; then
      CURRENT=$(jget "$TMP/body" current 2>/dev/null) || CURRENT=
      $restarted || CURRENT=
    else
      CURRENT=
    fi
    if $restarted && [ "$CODE" = 200 ] && [ -n "$CURRENT" ]; then
      if [ "$CURRENT" = "$next" ]; then rolled=true; else other=$CURRENT; fi
      break
    fi
    [ $(($(date +%s) - start)) -lt "$rollback_wait" ] || break
    sleep "$poll"
  done
  WAY_BACK="sh scripts/box.sh flash --image radio --keep-storage over USB"
  if $rolled; then
    check 6 rollback OK "the bootloader rolled the panicking image back to $next"
  elif [ -n "$other" ]; then
    check 6 rollback KO "the box answers $other, not $next; the way back: $WAY_BACK"
    : >"$SDIR/stop"
  elif $restarted; then
    check 6 rollback KO "the box restarted but did not answer within $rollback_wait s; the way back: $WAY_BACK"
    : >"$SDIR/stop"
  else
    check 6 rollback KO "no restart seen within $rollback_wait s (the old token is still valid); the way back: $WAY_BACK"
    : >"$SDIR/stop"
  fi
}

# GET /system/version with the token: CODE, CURRENT.
get_version() {
  http GET /system/version -H "@$AUTH_FILE" --max-time 10
  CURRENT=
  if [ "$CODE" = 200 ]; then CURRENT=$(jget "$TMP/body" current 2>/dev/null) || CURRENT=; fi
}

# ---- reset (J4, spec 029) ----

group_reset() {
  group_login || return
  http POST /integrations/solite/permit-join -H "@$AUTH_FILE" -H "$JSON" -d '{"time":0}'
  if [ "$CODE" = 202 ]; then check 0 "pairing closed" OK; else check 0 "pairing closed" KO "answered $CODE: $(errtext)"; fi
  http POST /system/factory-reset -H "@$AUTH_FILE" -H "$JSON" -d '{}'
  reset=$CODE
  if [ "$reset" = 204 ]; then check 1 "factory reset" OK; else check 1 "factory reset" KO "answered $reset: $(errtext)"; fi
  http GET /auth/status
  if [ "$CODE" = 200 ] && [ "$(jget "$TMP/body" setupRequired 2>/dev/null)" = true ]; then
    check 2 "setup required again" OK
  else
    check 2 "setup required again" KO "GET /auth/status answered $CODE, setupRequired true expected"
  fi
  pick_password
  if [ -z "$PW" ]; then
    check 3 "setup again" KO "no password"
  else
    attempt setup "$PW"
    if [ "$CODE" = 201 ]; then check 3 "setup again" OK; else check 3 "setup again" KO "setup answered $CODE: $(errtext)"; fi
  fi
}

# ---- the pairing window, before measure-api.sh (B6) ----

group_permit() {
  group_login || return
  check 0 "pairing closed" OK
}

# ---- script recipe (spec 041, R7; spec 046) ----

# The garage's own `motion-light` instance, from the list of instances on the standard input: its
# id, its first light's id, and its timeout in seconds (a number of milliseconds, or "10s", "10m",
# "1h"; 600 when it says nothing; empty when the text cannot be read, as "1h30m": the off check is
# then skipped). Nothing when no enabled instance is there.
GARAGE_PY=$(cat <<'PY'
import sys, json, re
for instance in json.load(sys.stdin):
    if instance.get("recipeId") != "motion-light" or not instance.get("enabled", True):
        continue
    params = instance.get("params") or {}
    lights = params.get("lights") or []
    timeout = params.get("timeout", "10m")
    seconds = 600
    if isinstance(timeout, (int, float)):
        seconds = int(timeout) // 1000
    elif isinstance(timeout, str):
        found = re.match(r"^\s*(\d+)\s*([smh])$", timeout)
        seconds = int(found.group(1)) * {"s": 1, "m": 60, "h": 3600}[found.group(2)] if found else ""
    else:
        seconds = ""
    print(instance["id"])
    print(lights[0] if lights else "")
    print(seconds)
    break
PY
)

# on, off or unknown: the light of an equipment (the file $1 holds its JSON): a data of category
# `light_state` first, else the alias `state`, as the script reads it.
LIGHT_PY=$(cat <<'PY'
import sys, json
try:
    equipment = json.load(open(sys.argv[1]))
except (OSError, ValueError):
    print("unknown")
    sys.exit(0)
bindings = equipment.get("dataBindings") or []
chosen = [b for b in bindings if b.get("category") == "light_state"] or [b for b in bindings if b.get("alias") == "state"]
if not chosen:
    print("unknown")
else:
    value = chosen[0].get("value")
    print("on" if value is True or str(value).upper() == "ON" else "off" if value is not None else "unknown")
PY
)

light_state() {
  http GET "/equipments/$1" -H "@$AUTH_FILE"
  if [ "$CODE" != 200 ]; then printf unknown; return; fi
  python3 -c "$LIGHT_PY" "$TMP/body"
}

# The recipe this run uploaded (`--recipe-file`) stays when an instance uses it, which the garage's
# instance does: a recipe an instance on the box uses is never uninstalled. Only an upload that no
# instance uses is taken away again, so that the box is left as it was found. A fresh login first:
# the access token may have expired during the wait.
script_cleanup() {
  [ -n "${RECIPE_UPLOADED:-}" ] || return 0
  relogin || :
  http GET /recipe-instances -H "@$AUTH_FILE"
  if [ "$CODE" != 200 ]; then
    check 7 "uploaded recipe" KO "GET /recipe-instances answered $CODE"
    return
  fi
  if grep -aq '"recipeId":"motion-light"' "$TMP/body"; then
    check 7 "uploaded recipe" OK "kept: an instance of motion-light uses it"
    return
  fi
  http POST /plugins/motion-light/uninstall -H "@$AUTH_FILE"
  case "$CODE" in
    200 | 204) RECIPE_UPLOADED=; check 7 "uploaded recipe" OK "no instance uses it: uninstalled again" ;;
    *) check 7 "uploaded recipe" KO "motion-light was not uninstalled ($CODE)" ;;
  esac
}

# The person's wave (a seam for the tests: SOLITE_SESSION_WAVE=auto asks nobody).
ask_wave() {
  [ "${SOLITE_SESSION_WAVE:-}" != auto ] || return 0
  (exec </dev/tty) 2>/dev/null || return 1
  printf '\nbox-session: %s (spec 041): wave at the garage motion sensor now, for a few seconds, then press Enter\n' "$GID" >/dev/tty
  IFS= read -r _wave </dev/tty || return 1
}

# Polls the light for at most $2 seconds until it is $1 (on or off); returns 0 when it was.
wait_light() {
  _want=$1 _max=$2
  _t=0
  while :; do
    [ "$(light_state "$LIGHT")" != "$_want" ] || return 0
    [ "$_t" -lt "$_max" ] || return 1
    sleep "${SOLITE_SESSION_SCRIPT_POLL:-1}"
    _t=$((_t + ${SOLITE_SESSION_SCRIPT_POLL:-1}))
  done
}

group_script() {
  group_login || return
  RECIPE_UPLOADED=
  # Whatever happens below, an upload no instance uses is taken away (a signal ends the shell
  # through `exit`).
  trap 'script_cleanup; rm -rf "$TMP"' EXIT

  # 1. The `motion-light` recipe on the box: it is installed, or, with `--recipe-file <file.srs>` (a
  # signed file), uploaded for this run; without it the group says so and stops.
  http GET /recipes -H "@$AUTH_FILE"
  if [ "$CODE" != 200 ]; then
    check 1 "script recipe on the box" KO "GET /recipes answered $CODE"
    return
  fi
  if ! grep -aq '"id":"motion-light"' "$TMP/body"; then
    if [ -z "${SOLITE_SESSION_RECIPE_FILE:-}" ]; then
      check 1 "script recipe on the box" skipped "not installed: --recipe-file"
      return
    fi
    if [ ! -f "$SOLITE_SESSION_RECIPE_FILE" ]; then
      check 1 "script recipe on the box" KO "the recipe file $SOLITE_SESSION_RECIPE_FILE does not exist"
      return
    fi
    http POST /plugins/install -H "@$AUTH_FILE" -H "Content-Type: application/octet-stream" --data-binary "@$SOLITE_SESSION_RECIPE_FILE"
    if [ "$CODE" != 200 ]; then
      check 1 "script recipe on the box" KO "uploading $SOLITE_SESSION_RECIPE_FILE answered $CODE: $(errtext)"
      return
    fi
    RECIPE_UPLOADED=1
    check 1 "script recipe on the box" OK "installed from $SOLITE_SESSION_RECIPE_FILE"
  else
    check 1 "script recipe on the box" OK "installed"
  fi

  # 2. The garage's own instance: it is the one the wave is checked on (nothing is copied, swapped
  # or deleted: the home is not touched).
  http GET /recipe-instances -H "@$AUTH_FILE"
  if [ "$CODE" != 200 ]; then
    check 2 "garage instance" KO "GET /recipe-instances answered $CODE"
    return
  fi
  _garage=$(python3 -c "$GARAGE_PY" <"$TMP/body" 2>/dev/null)
  GARAGE=$(printf '%s\n' "$_garage" | sed -n 1p)
  LIGHT=$(printf '%s\n' "$_garage" | sed -n 2p)
  _timeout=$(printf '%s\n' "$_garage" | sed -n 3p)
  if [ -z "$GARAGE" ]; then
    check 2 "garage instance" skipped "the box has no enabled motion-light instance (the garage's)"
    return
  fi
  if [ -z "$LIGHT" ]; then
    check 2 "garage instance" KO "the motion-light instance $GARAGE has no light"
    return
  fi
  if [ -n "$_timeout" ]; then
    check 2 "garage instance" OK "motion-light $GARAGE, light $LIGHT, timeout ${_timeout}s"
  else
    check 2 "garage instance" OK "motion-light $GARAGE, light $LIGHT, timeout unreadable: the off check is skipped"
  fi

  # 3. The wave: the light on within 10 s of it, then off within the instance's timeout (and a
  # margin) of the light being seen on (the timeout runs from the last motion, which is no later
  # than that moment).
  _off_wait=${SOLITE_SESSION_OFF_WAIT:-}
  [ -n "$_off_wait" ] || [ -z "$_timeout" ] || _off_wait=$((_timeout + 30))
  if ! ask_wave; then
    check 3 "light on" KO "no terminal to ask the person at"
    return
  fi
  if wait_light on "${SOLITE_SESSION_ON_WAIT:-10}"; then
    check 3 "light on" OK "on within ${SOLITE_SESSION_ON_WAIT:-10} s of the wave"
  else
    check 3 "light on" KO "the light $LIGHT was not on ${SOLITE_SESSION_ON_WAIT:-10} s after the wave"
    return
  fi
  if [ -z "$_off_wait" ]; then
    check 4 "light off" skipped "the instance's timeout cannot be read, so how long to wait is unknown"
  elif wait_light off "$_off_wait"; then
    check 4 "light off" OK "off within $_off_wait s of being seen on"
  else
    check 4 "light off" KO "the light $LIGHT was still on $_off_wait s after it was seen on"
    return
  fi

  # 4. The box prints its `heap:` and `scripts:` lines every 240 s: the session's serial log, read
  # by the step once this ends, holds the next ones if the instance stays that long. The
  # wait is the whole 240 s and not "at most": `box.sh saves` writes the session's log file only
  # when the session ends (`keep_log`, an `atexit` of its serial reader), and gives this command
  # nothing to read it by, so the `scripts:` line cannot be polled from here to stop early.
  sleep "${SOLITE_SESSION_SCRIPT_WAIT:-240}"
  check 5 "instance kept for the lines" OK "${SOLITE_SESSION_SCRIPT_WAIT:-240} s"
}

if [ "${1:-}" = --group ]; then
  GROUP=${2:-}
  case "$GROUP" in
    auth) GID=B4 ;;
    home) GID=B7 ;;
    update) GID=B8 ;;
    reset) GID=B10 ;;
    permit) GID=B6 ;;
    script) GID=B11 ;;
    *) echo "box-session: --group takes auth, home, update, reset, permit or script" >&2; exit 2 ;;
  esac
  GID=${SOLITE_SESSION_ID:-$GID}
  SDIR=${SOLITE_SESSION_DIR:-}
  ADDR=${SOLITE_BOX_ADDRESS:-}
  if [ -z "$SDIR" ] || [ -z "$ADDR" ]; then
    echo "box-session: --group needs SOLITE_SESSION_DIR and SOLITE_BOX_ADDRESS" >&2
    exit 2
  fi
  API=http://$ADDR/api/v1
  PWFILE=${SOLITE_PASSWORD_FILE:-$SDIR/password}
  TMP=$(mktemp -d "$SDIR/group.XXXXXX") || exit 2
  trap 'rm -rf "$TMP"' EXIT
  trap 'exit 143' TERM
  trap 'exit 130' INT HUP
  AUTH_FILE=$TMP/auth
  : >"$AUTH_FILE"
  TOKEN=
  RTOKEN=
  ATTEMPTS=0
  GKO=0
  "group_$GROUP"
  [ "$GKO" = 0 ]
  exit $?
fi

# ---------------------------------------------------------------------------------------------
# The outer run
# ---------------------------------------------------------------------------------------------

usage() {
  echo "usage: sh scripts/box-session.sh [--yes-flash] [--backup <file>] [--factory-reset] [--with-gestures] [--recipe-file <file.srs>] [--port <device>] [--address <address>[:<port>]]" >&2
  echo "       sh scripts/box-session.sh --api-only <address>[:<port>] [--factory-reset]" >&2
  [ -z "${1:-}" ] || echo "box-session: $1" >&2
  exit 2
}

YES_FLASH=false
BACKUP=${SOLITE_BOX_BACKUP:-}
FACTORY_RESET=false
GESTURES=false
RECIPE_FILE=${SOLITE_SESSION_RECIPE_FILE:-}
PORT=
ADDRESS_OPT=${SOLITE_BOX_ADDRESS:-}
API_ONLY=false
API_ADDR=
while [ $# -gt 0 ]; do
  case "$1" in
    --yes-flash) YES_FLASH=true; shift ;;
    --backup) [ $# -ge 2 ] || usage "--backup needs a file"; BACKUP=$2; shift 2 ;;
    --factory-reset) FACTORY_RESET=true; shift ;;
    --with-gestures) GESTURES=true; shift ;;
    --recipe-file) [ $# -ge 2 ] || usage "--recipe-file needs a file"; RECIPE_FILE=$2; shift 2 ;;
    --port) [ $# -ge 2 ] || usage "--port needs a device"; PORT=$2; shift 2 ;;
    --address) [ $# -ge 2 ] || usage "--address needs an address"; ADDRESS_OPT=$2; shift 2 ;;
    --api-only) [ $# -ge 2 ] || usage "--api-only needs an address"; API_ONLY=true; API_ADDR=$2; shift 2 ;;
    *) usage "unknown option $1" ;;
  esac
done
if [ -n "$RECIPE_FILE" ]; then
  case "$RECIPE_FILE" in /*) ;; *) RECIPE_FILE=$CALLER_DIR/$RECIPE_FILE ;; esac
  [ -f "$RECIPE_FILE" ] || usage "recipe file not found: $RECIPE_FILE"
  SOLITE_SESSION_RECIPE_FILE=$RECIPE_FILE
  export SOLITE_SESSION_RECIPE_FILE
else
  unset SOLITE_SESSION_RECIPE_FILE
fi
$API_ONLY && $YES_FLASH && usage "--yes-flash cannot go with --api-only"
if $FACTORY_RESET && ! $YES_FLASH && ! $API_ONLY; then
  usage "--factory-reset needs --yes-flash or --api-only"
fi
if $YES_FLASH; then
  [ -n "$BACKUP" ] || usage "--yes-flash needs --backup <file> (the original firmware's backup)"
  case "$BACKUP" in /*) ;; *) BACKUP=$CALLER_DIR/$BACKUP ;; esac
  [ -f "$BACKUP" ] || usage "backup not found: $BACKUP"
  (exec </dev/tty) 2>/dev/null || usage "--yes-flash is typed by the user in a terminal"
fi
if $API_ONLY; then
  case "$API_ADDR" in
    '' | *[!0-9]*) ;;
    *) API_ADDR=127.0.0.1:$API_ADDR ;;
  esac
  case "$API_ADDR" in
    '[::1]' | '[::1]:'*) _ahost=::1 ;;
    '::1') _ahost=::1; API_ADDR='[::1]:80' ;;
    *:*) _ahost=${API_ADDR%%:*} ;;
    *) _ahost=$API_ADDR; API_ADDR=$API_ADDR:80 ;;
  esac
  case "$_ahost" in
    127.0.0.1 | localhost | ::1) ;;
    *)
      echo "box-session: --api-only is for the Mac host (127.0.0.1, localhost, ::1): $_ahost refused" >&2
      exit 2 ;;
  esac
fi

# The run folder, never inside the repository.
SROOT=${SOLITE_SESSION_ROOT:-${TMPDIR:-/tmp}/solite-box-session}
case "$SROOT" in /*) ;; *) SROOT=$CALLER_DIR/$SROOT ;; esac
_sroot_real=$(cd "$SROOT" 2>/dev/null && pwd -P) || _sroot_real=$SROOT
case "$SROOT/" in
  "$ROOT"/*) usage "SOLITE_SESSION_ROOT is inside the repository: $SROOT" ;;
esac
case "$_sroot_real/" in
  "$ROOT"/*) usage "SOLITE_SESSION_ROOT is inside the repository: $SROOT" ;;
esac
RUN=$SROOT/$(date +%Y%m%d-%H%M%S)
[ ! -e "$RUN" ] || RUN=$RUN-$$
mkdir -p "$SROOT" && mkdir -m 700 "$RUN" || { echo "box-session: cannot create $RUN" >&2; exit 2; }
SESSION_LOG=$RUN/session.log
: >"$SESSION_LOG"

BOX=${SOLITE_BOX_SCRIPT:-$ROOT/scripts/box.sh}
BOXC="sh $BOX"
if [ -n "${SOLITE_HOST_BIN:-}" ]; then HOSTC=$SOLITE_HOST_BIN; else HOSTC="cargo run -q --manifest-path $ROOT/Cargo.toml -p solite-host --"; fi
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
PORTARGS=
[ -z "$PORT" ] || PORTARGS="--port $PORT"
RADIO_DIR=${SOLITE_RADIO_TARGET_DIR:-$ROOT/firmware/target/radio/xtensa-esp32-espidf/release}

SOLITE_SESSION_DIR=$RUN
export SOLITE_SESSION_DIR SOLITE_USER
if $API_ONLY; then
  PWFILE=${SOLITE_PASSWORD_FILE:-$RUN/password}
  SOLITE_SESSION_API_ONLY=1
  SOLITE_SESSION_RATE_WAIT=${SOLITE_SESSION_RATE_WAIT:-61}
  export SOLITE_SESSION_API_ONLY SOLITE_SESSION_RATE_WAIT
  SOLITE_BOX_ADDRESS=$API_ADDR
  export SOLITE_BOX_ADDRESS
else
  PWFILE=${SOLITE_PASSWORD_FILE:-$HOME/.config/solite/box-password}
  # Only --api-only sets it; a leftover in the environment must not skip the box-only checks.
  # SOLITE_SESSION_TEST_HOST is the tests' seam: a box run whose "box" is a Mac host.
  unset SOLITE_SESSION_API_ONLY
  if [ -n "${SOLITE_SESSION_TEST_HOST:-}" ]; then
    echo "box-session: SOLITE_SESSION_TEST_HOST set: box-only checks skipped (unset it for a real box run)"
    SOLITE_SESSION_API_ONLY=1
    export SOLITE_SESSION_API_ONLY
  fi
fi
SOLITE_PASSWORD_FILE=$PWFILE
export SOLITE_PASSWORD_FILE

CHILD=
STOPPED=
N_OK=0
N_KO=0
N_SKIP=0

cleanup() {
  rm -f "$RUN/update.key" "$RUN/tokens" "$RUN/request"
}
on_signal() {
  [ -z "$CHILD" ] || kill "$CHILD" 2>/dev/null
  exit "$1"
}
trap cleanup EXIT
trap 'on_signal 143' TERM
trap 'on_signal 130' INT HUP

# A step's line: $1 its id, $2 its name, $3 OK|KO|skipped, $4 the detail or the reason. Counted.
report() {
  if [ -n "${4:-}" ]; then
    _line="box-session: $1 $2: $3 ($4)"
  else
    _line="box-session: $1 $2: $3"
  fi
  printf '%s\n' "$_line"
  printf '%s\n' "$_line" >>"$SESSION_LOG"
  eval "RES_$1=$3"
  case "$3" in
    OK) N_OK=$((N_OK + 1)) ;;
    KO) N_KO=$((N_KO + 1)) ;;
    *) N_SKIP=$((N_SKIP + 1)) ;;
  esac
}

# Runs a command with its whole output in a file (appended to the log); the run can be interrupted
# while it waits. $1 the file; returns the command's exit code.
run() {
  _out=$1
  shift
  "$@" >"$_out" 2>&1 </dev/null &
  CHILD=$!
  wait "$CHILD"
  _rc=$?
  CHILD=
  cat "$_out" >>"$SESSION_LOG" 2>/dev/null
  return "$_rc"
}

# The last line of a command's output that tells why it failed: a `box:` line that is not
# progress, else the last line, else the exit code ($1 the output file, $2 the exit code).
reason_of() {
  _r=$(grep -a '^box: ' "$1" 2>/dev/null | grep -avE '^box: (saves held|saves flushed|serial log|capturing|flash: [0-9]+ writes since boot$)' | tail -n 1 | sed 's/^box: //')
  [ -n "$_r" ] || _r=$(grep -av '^$' "$1" 2>/dev/null | tail -n 1 | cut -c1-200)
  printf '%s' "${_r:-exit $2}"
}

# True when the step may run; else its line says why. $1 id, $2 name, $3 `box` (needs the box) or
# `api` (also runs with --api-only), $4 `w` (flashes or writes) or `r` (read-only).
precheck() {
  _id=$1 _name=$2 _kind=$3 _writes=$4
  if $API_ONLY; then
    if [ "$_kind" = box ]; then report "$_id" "$_name" skipped "box only"; return 1; fi
  elif [ "$_writes" = w ] && ! $YES_FLASH; then
    report "$_id" "$_name" skipped "needs --yes-flash"
    return 1
  fi
  if [ -n "$STOPPED" ] && { [ "$_kind" = box ] || ! $API_ONLY; }; then
    report "$_id" "$_name" skipped "stopped after $STOPPED"
    return 1
  fi
  return 0
}

# The outcome of an earlier step ($1 its id).
outcome() { eval "printf '%s' \"\${RES_$1:-skipped}\""; }

# Skips the step when an earlier one is not OK: $1 id, $2 name, then the steps it depends on.
after() {
  _id=$1 _name=$2
  shift 2
  for _dep in "$@"; do
    _o=$(outcome "$_dep")
    if [ "$_o" != OK ]; then report "$_id" "$_name" skipped "after $_dep $_o"; return 1; fi
  done
  return 0
}

# "An account usable": the auth group's check 2 wrote the file.
account_usable() {
  if [ ! -f "$RUN/account-usable" ]; then report "$1" "$2" skipped "after B4: no account"; return 1; fi
  return 0
}

# The password for the steps outside the groups: SOLITE_PASSWORD, else the password file's.
refresh_password() {
  if [ -z "${SOLITE_PASSWORD:-}" ] && [ -f "$PWFILE" ]; then
    SOLITE_PASSWORD=$(head -n 1 "$PWFILE")
    export SOLITE_PASSWORD
  fi
}

# ---- the steps ----

BASE_VERSION=
NEXT_VERSION=
PUBKEY=

step_p1() {
  _missing=
  command -v curl >/dev/null 2>&1 || _missing="$_missing curl"
  command -v python3 >/dev/null 2>&1 || _missing="$_missing python3"
  $API_ONLY || [ -f "$BOX" ] || _missing="$_missing box.sh($BOX)"
  if [ -n "$_missing" ]; then
    report P1 preflight KO "missing:$_missing"
    return
  fi
  _warn=
  if $YES_FLASH && git -C "$ROOT" rev-parse --git-dir >/dev/null 2>&1; then
    head_version=$(git -C "$ROOT" show HEAD:Cargo.toml 2>/dev/null | sed -n '/^\[workspace.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' | head -n 1)
    if [ -n "$(git -C "$ROOT" status --porcelain -- crates firmware 2>/dev/null)" ] || [ "$head_version" != "$(fw_version)" ]; then
      _warn="warning: uncommitted changes: the update images are built from HEAD"
    fi
  fi
  report P1 preflight OK "$_warn"
}

# The version with its patch number raised by $2 (x.y.z only); empty when it is not x.y.z.
bump() {
  case "$1" in
    *[!0-9.]* | '' | .* | *. | *..*) return 0 ;;
  esac
  _x=${1%%.*}
  _rest=${1#*.}
  [ "$_rest" != "$1" ] || return 0
  _y=${_rest%%.*}
  _z=${_rest#*.}
  [ "$_z" != "$_rest" ] || return 0
  case "$_z" in *.*) return 0 ;; esac
  printf '%s.%s.%s' "$_x" "$_y" "$((_z + $2))"
}

fw_fail() { echo "$1"; exit 1; }
. "$ROOT/scripts/lib/firmware.sh"

step_p2() {
  if ! $YES_FLASH && ! $API_ONLY; then report P2 "update key" skipped "needs --yes-flash"; return; fi
  rm -f "$RUN/update.key"
  run "$RUN/p2.out" $HOSTC --update-keygen "$RUN/update.key"
  _rc=$?
  PUBKEY=$(sed -n 's/^public key: \([0-9a-f]\{64\}\).*/\1/p' "$RUN/p2.out" | tail -n 1)
  if [ "$_rc" -ne 0 ] || [ -z "$PUBKEY" ]; then
    report P2 "update key" KO "$(reason_of "$RUN/p2.out" "$_rc")"
    return
  fi
  report P2 "update key" OK "public key ${PUBKEY%"${PUBKEY#????????}"}..."
}

# The sed-free change of a `version` line: $1 the file, $2 the section, $3 the version.
set_version() {
  awk -v sect="[$2]" -v ver="$3" '
    $0 == sect { in_s = 1; print; next }
    /^\[/ { in_s = 0 }
    in_s && !done && /^version *= *"/ { print "version = \"" ver "\""; done = 1; next }
    { print }
    END { exit done ? 0 : 1 }' "$1" >"$1.new" && mv "$1.new" "$1" && return 0
  rm -f "$1.new"
  echo "no version line under [$2] in ${1#"$SROOT"/}"
  return 1
}

# Exports the recipe key's public half (spec 042 R1.5) to the build when `release/recipe-key.pub`
# exists (the user makes it; this repository never holds the private half): the image then installs
# recipes signed with it, and a build without the file answers every install with 503.
recipe_key_env() {
  _rk=$(tr -d ' \t\r\n' <"$ROOT/release/recipe-key.pub" 2>/dev/null) || _rk=
  [ -z "$_rk" ] || export SOLITE_RECIPE_PUBLIC_KEY=$_rk
  return 0
}

# The script's own builder (spec 033 R5.3): $1 base|next|rollback, $2 the version, $3 the public key
# in hex, $4 the application image to write. Prints why it failed.
own_builder() {
  _kind=$1 _version=$2 _key=$3 _out=$4
  if [ "$_kind" = base ]; then
    [ -f "$ROOT/ui/dist/index.html" ] || { echo "ui/dist is missing: cd ui && npm run build"; return 1; }
    [ -f "$HOME/export-esp.sh" ] || { echo "the ESP32 toolchain is not set up (espup install, see README.md)"; return 1; }
    (
      . "$HOME/export-esp.sh"
      cd "$ROOT/firmware" || exit 1
      recipe_key_env
      SOLITE_UPDATE_PUBLIC_KEY=$_key CARGO_TARGET_DIR=target/radio ESP_IDF_SDKCONFIG_DEFAULTS="sdkconfig.defaults;sdkconfig.radio.defaults" \
        cargo build --release --features radio || exit 1
      fw_find_tools
      fw_build_table "$ROOT/firmware/partitions.csv" "$RUN/table.bin"
      fw_elf2image "$ROOT/firmware/target/radio/xtensa-esp32-espidf/release/solite-firmware" "$_out"
    )
    return $?
  fi
  _tree=$SROOT/box-session-tree
  rm -rf "$_tree"
  mkdir -p "$_tree" || return 1
  git -C "$ROOT" archive HEAD | tar -x -C "$_tree" || { echo "cannot copy HEAD into $_tree"; return 1; }
  set_version "$_tree/Cargo.toml" workspace.package "$_version" || return 1
  set_version "$_tree/firmware/Cargo.toml" package "$_version" || return 1
  if [ "$_kind" = rollback ]; then
    awk '{ print } /^fn main\(\) \{$/ && !done { print "    if std::hint::black_box(true) { panic!(\"box-session: rollback image\") }"; done = 1 }' \
      "$_tree/firmware/src/main.rs" >"$_tree/firmware/src/main.rs.new" && mv "$_tree/firmware/src/main.rs.new" "$_tree/firmware/src/main.rs"
    grep -q 'box-session: rollback image' "$_tree/firmware/src/main.rs" || { echo "no 'fn main() {' line in firmware/src/main.rs"; return 1; }
  fi
  [ -f "$HOME/export-esp.sh" ] || { echo "the ESP32 toolchain is not set up (espup install, see README.md)"; return 1; }
  [ -d "$ROOT/firmware/.embuild" ] && ln -sfn "$ROOT/firmware/.embuild" "$_tree/firmware/.embuild"
  (
    . "$HOME/export-esp.sh"
    cd "$_tree/firmware" || exit 1
    recipe_key_env
    SOLITE_UPDATE_PUBLIC_KEY=$_key SOLITE_UI_DIST=$ROOT/ui/dist CARGO_TARGET_DIR=$SROOT/target \
      ESP_IDF_SDKCONFIG_DEFAULTS="sdkconfig.defaults;sdkconfig.radio.defaults" cargo build --release --features radio || exit 1
    fw_find_tools
    fw_build_table "$_tree/firmware/partitions.csv" "$RUN/table.bin"
    fw_elf2image "$SROOT/target/xtensa-esp32-espidf/release/solite-firmware" "$_out"
  )
}

# $1 base|next|rollback, $2 the version, $3 the image to write; the builder's output is in $RUN/<kind>.out.
build() {
  if [ -n "${SOLITE_SESSION_BUILDER:-}" ]; then
    run "$RUN/build-$1.out" sh "$SOLITE_SESSION_BUILDER" "$1" "$2" "$PUBKEY" "$3"
  else
    run "$RUN/build-$1.out" own_builder "$1" "$2" "$PUBKEY" "$3"
  fi
}

step_p3() {
  if $API_ONLY; then report P3 "radio image with the key" skipped "box only"; return; fi
  if ! $YES_FLASH; then report P3 "radio image with the key" skipped "needs --yes-flash"; return; fi
  after P3 "radio image with the key" P2 || return
  build base "$BASE_VERSION" "$RUN/base.bin"
  _rc=$?
  if [ "$_rc" -ne 0 ] || [ ! -s "$RUN/base.bin" ]; then
    report P3 "radio image with the key" KO "$(reason_of "$RUN/build-base.out" "$_rc")"
    return
  fi
  report P3 "radio image with the key" OK
}

# A file with one signature byte flipped: $1 the signed file, $2 the copy. The signature is at 64.
flip_signature() {
  python3 -c '
import sys
data = bytearray(open(sys.argv[1], "rb").read())
data[64] ^= 0xFF
open(sys.argv[2], "wb").write(data)' "$1" "$2"
}

step_p4() {
  if ! $YES_FLASH && ! $API_ONLY; then report P4 "update images" skipped "needs --yes-flash"; return; fi
  after P4 "update images" P2 || return
  if [ -z "$NEXT_VERSION" ]; then
    report P4 "update images" KO "cannot read or raise the version (Cargo.toml's [workspace.package])"
    return
  fi
  rm -f "$RUN/next.sup" "$RUN/rollback.sup" "$RUN/bad.sup"
  if $API_ONLY; then
    printf 'box-session image' | head -c 16 >"$RUN/next.bin"
    run "$RUN/p4.out" $HOSTC --update-sign "$RUN/next.bin" --version "$NEXT_VERSION" --key "$RUN/update.key" --out "$RUN/next.sup"
    _rc=$?
  else
    _rb=$(bump "$BASE_VERSION" 2)
    build next "$NEXT_VERSION" "$RUN/next.bin"
    _rc=$?
    if [ "$_rc" -ne 0 ] || [ ! -s "$RUN/next.bin" ]; then
      report P4 "update images" KO "next version: $(reason_of "$RUN/build-next.out" "$_rc")"
      return
    fi
    build rollback "$_rb" "$RUN/rollback.bin"
    _rc=$?
    if [ "$_rc" -ne 0 ] || [ ! -s "$RUN/rollback.bin" ]; then
      report P4 "update images" KO "rollback image: $(reason_of "$RUN/build-rollback.out" "$_rc")"
      return
    fi
    run "$RUN/p4.out" $HOSTC --update-sign "$RUN/next.bin" --version "$NEXT_VERSION" --key "$RUN/update.key" --out "$RUN/next.sup"
    _rc=$?
    if [ "$_rc" -eq 0 ]; then
      run "$RUN/p4.out" $HOSTC --update-sign "$RUN/rollback.bin" --version "$_rb" --key "$RUN/update.key" --out "$RUN/rollback.sup"
      _rc=$?
    fi
  fi
  if [ "$_rc" -ne 0 ] || [ ! -f "$RUN/next.sup" ]; then
    report P4 "update images" KO "signature: $(reason_of "$RUN/p4.out" "$_rc")"
    return
  fi
  if ! flip_signature "$RUN/next.sup" "$RUN/bad.sup"; then
    report P4 "update images" KO "cannot flip a signature byte"
    return
  fi
  printf '%s\n' "$NEXT_VERSION" >"$RUN/next.version"
  report P4 "update images" OK "next $NEXT_VERSION"
}

step_p5() {
  _elf=$RADIO_DIR/solite-firmware
  if [ ! -f "$_elf" ]; then report P5 "image size" skipped "no radio build"; return; fi
  _line=$( (
    fw_find_tools
    fw_build_table "$ROOT/firmware/partitions.csv" "$RUN/table.bin"
    fw_elf2image "$_elf" "$RUN/size.bin"
    fw_size_line radio "$RUN/size.bin"
  ) 2>&1 | tail -n 1)
  case "$_line" in
    "radio: "*" KB, "*" % of the "*) report P5 "image size" OK "${_line#radio: }" ;;
    *) report P5 "image size" KO "$_line" ;;
  esac
}

# The last line of the file $1 that holds $2.
logline() { grep -a "$2" "$1" 2>/dev/null | tail -n 1; }

step_b1() {
  precheck B1 identify box r || return
  run "$RUN/b1.out" $BOXC identify $PORTARGS
  _rc=$?
  if [ "$_rc" -eq 0 ]; then
    report B1 identify OK
  else
    report B1 identify KO "$(reason_of "$RUN/b1.out" "$_rc")"
    STOPPED=B1
  fi
}

step_b2() {
  precheck B2 flash box w || return
  after B2 flash B1 P3 || return
  run "$RUN/b2.out" $BOXC flash --image radio --keep-storage --backup "$BACKUP" $PORTARGS
  _rc=$?
  if [ "$_rc" -eq 0 ]; then
    report B2 flash OK "radio image, storage kept"
  else
    report B2 flash KO "$(reason_of "$RUN/b2.out" "$_rc")"
    STOPPED=B2
  fi
}

step_b3() {
  precheck B3 "boot and memory" box r || return
  after B3 "boot and memory" B1 || return
  if $YES_FLASH; then after B3 "boot and memory" B2 || return; fi
  run "$RUN/b3.out" $BOXC log --image radio --budget 1 $PORTARGS
  _rc=$?
  _log=$(sed -n 's/^box: serial log: //p' "$RUN/b3.out" | tail -n 1)
  if [ "$_rc" -ne 0 ]; then
    report B3 "boot and memory" KO "$(reason_of "$RUN/b3.out" "$_rc")"
    ! $YES_FLASH || STOPPED=B3
    return
  fi
  if [ -z "$_log" ] || [ ! -f "$_log" ]; then
    report B3 "boot and memory" KO "no serial log in box.sh's output"
    return
  fi
  _mem=$(logline "$_log" 'memory-network: ' | sed 's/^.*memory-network: //')
  _heap=$(logline "$_log" 'heap: internal' | sed 's/^.*heap: //')
  _stor=$(logline "$_log" 'storage: /data, ' | sed 's/^.*storage: //')
  report B3 "boot and memory" OK "${_mem:-no memory-network line}; ${_heap:-no heap line}; ${_stor:-no storage line}"
}

# Prints a group's check lines and sums them up as the step's line: $1 id, $2 name, $3 the output
# file, $4 the exit code (SV_OVER: the budget's refusal, when the group ran inside box.sh saves).
finish_group() {
  _id=$1 _name=$2 _out=$3 _rc=$4
  grep -a '^box-session: ' "$_out" 2>/dev/null
  _all=$(grep -ac '^box-session: ' "$_out")
  _ko=$(grep -ac '^box-session: [^ ]* .*: KO' "$_out")
  if [ -n "$SV_OVER" ]; then
    report "$_id" "$_name" KO "$SV_OVER"
    STOPPED=$_id
  elif [ "$_all" -eq 0 ]; then
    report "$_id" "$_name" KO "no check ran: $(reason_of "$_out" "$_rc")"
  elif [ "$_ko" -gt 0 ]; then
    report "$_id" "$_name" KO "$_ko of $_all checks"
  elif [ "$_rc" -ne 0 ]; then
    report "$_id" "$_name" KO "$(reason_of "$_out" "$_rc")"
  else
    report "$_id" "$_name" OK "$_all checks"
  fi
}

# Runs a command inside `box.sh saves --budget <n>` (a box run) or straight (--api-only). $1 the
# step's id, $2 the budget; the output is in $RUN/<id>.out. Sets SV_RC, SV_OVER, SV_LOG.
via_saves() {
  _id=$1 _budget=$2
  shift 2
  SV_OUT=$RUN/$_id.out
  SOLITE_SESSION_ID=$_id
  export SOLITE_SESSION_ID
  if $API_ONLY; then
    run "$SV_OUT" "$@"
  else
    run "$SV_OUT" $BOXC saves --budget "$_budget" $PORTARGS -- "$@"
  fi
  SV_RC=$?
  SV_OVER=
  SV_LOG=
  if ! $API_ONLY; then
    SV_OVER=$(grep -a 'over the budget of' "$SV_OUT" | tail -n 1 | sed 's/^box: //')
    _addr=$(sed -n 's|^box: saves held; serving at http://\([^/]*\)/.*|\1|p' "$SV_OUT" | tail -n 1)
    [ -z "$_addr" ] || printf '%s\n' "$_addr" >"$RUN/address"
    SV_LOG=$(sed -n 's/^box: serial log: //p' "$SV_OUT" | tail -n 1)
  fi
}

step_b4() {
  precheck B4 "auth (J4)" api w || return
  $API_ONLY || after B4 "auth (J4)" B3 || return
  refresh_password
  via_saves B4 3 sh "$SELF" --group auth
  B4_LOG=$SV_LOG
  finish_group B4 "auth (J4)" "$SV_OUT" "$SV_RC"
  refresh_password
}

step_b5() {
  precheck B5 "second boot (021, 029)" box w || return
  if [ "$(outcome B4)" = skipped ]; then report B5 "second boot (021, 029)" skipped "after B4 skipped"; return; fi
  if [ -z "${B4_LOG:-}" ] || [ ! -f "$B4_LOG" ]; then
    report B5 "second boot (021, 029)" KO "no serial log of B4's session"
    return
  fi
  _why=
  grep -aq 'network: resumed' "$B4_LOG" || _why="no 'network: resumed' line"
  _version=$(fw_version)
  _banners=$(grep -acF "solite $_version" "$B4_LOG")
  [ "$_banners" -eq 1 ] || _why="${_why:+$_why; }the banner shows $_banners times, once expected"
  if [ -f "$RUN/tokens" ] && grep -aqFf "$RUN/tokens" "$B4_LOG"; then _why="${_why:+$_why; }a token is in the serial log"; fi
  refresh_password
  if [ -n "${SOLITE_PASSWORD:-}" ] && grep -aqF -- "$SOLITE_PASSWORD" "$B4_LOG"; then _why="${_why:+$_why; }the password is in the serial log"; fi
  if [ -z "$_why" ]; then
    report B5 "second boot (021, 029)" OK "network resumed, one banner, no token or password in the log"
  else
    report B5 "second boot (021, 029)" KO "$_why"
  fi
}

step_b6() {
  precheck B6 "17 requests (025)" api w || return
  account_usable B6 "17 requests (025)" || return
  refresh_password
  via_saves B6 5 sh -c 'sh "$1" --group permit; sh "$2" "$SOLITE_BOX_ADDRESS"' box-session "$SELF" "$ROOT/scripts/measure-api.sh"
  grep -a '^box-session: ' "$SV_OUT"
  _measure=$(grep -a '^measure: 17 requests' "$SV_OUT" | tail -n 1)
  if [ -n "$SV_OVER" ]; then
    report B6 "17 requests (025)" KO "$SV_OVER"
    STOPPED=B6
  elif [ "$SV_RC" -eq 0 ] && [ -n "$_measure" ]; then
    report B6 "17 requests (025)" OK "${_measure#measure: }"
  else
    _why=$(grep -aE '^(measure|measure-api):' "$SV_OUT" | tail -n 1)
    report B6 "17 requests (025)" KO "${_why:-$(reason_of "$SV_OUT" "$SV_RC")}"
  fi
}

step_b7() {
  precheck B7 "home (026, 027, 028, 023)" api w || return
  account_usable B7 "home (026, 027, 028, 023)" || return
  refresh_password
  via_saves B7 4 sh "$SELF" --group home
  finish_group B7 "home (026, 027, 028, 023)" "$SV_OUT" "$SV_RC"
}

step_b8() {
  precheck B8 "update (J5, 032)" api w || return
  after B8 "update (J5, 032)" P4 || return
  account_usable B8 "update (J5, 032)" || return
  if $API_ONLY; then
    _addr=$API_ADDR
  else
    _addr=${ADDRESS_OPT:-$(cat "$RUN/address" 2>/dev/null)}
  fi
  if [ -z "$_addr" ]; then
    report B8 "update (J5, 032)" KO "no address of the box: give --address"
    return
  fi
  refresh_password
  # The box may still be finishing B7's flush or the serial port's reset: wait for it to answer.
  _waited=0
  _bw=${SOLITE_SESSION_BOOT_WAIT:-180}
  _pw=${SOLITE_SESSION_POLL:-5}
  while :; do
    _st=$(curl -s -o /dev/null -w '%{http_code}' --max-time 5 "http://$_addr/api/v1/auth/status" 2>/dev/null)
    [ "$_st" = 200 ] && break
    [ "$_waited" -lt "$_bw" ] || break
    sleep "$_pw"
    _waited=$((_waited + _pw))
  done
  SOLITE_SESSION_ID=B8
  export SOLITE_SESSION_ID
  SV_OVER=
  run "$RUN/B8.out" env "SOLITE_BOX_ADDRESS=$_addr" sh "$SELF" --group update
  _rc=$?
  finish_group B8 "update (J5, 032)" "$RUN/B8.out" "$_rc"
  if [ -f "$RUN/stop" ]; then STOPPED=B8; fi
}

step_b9() {
  precheck B9 "memory of the updated image" box w || return
  after B9 "memory of the updated image" B8 || return
  run "$RUN/b9.out" $BOXC log --raw --seconds 300 --budget 1 $PORTARGS
  _rc=$?
  _log=$(sed -n 's/^box: serial log: //p' "$RUN/b9.out" | tail -n 1)
  if [ "$_rc" -ne 0 ]; then
    report B9 "memory of the updated image" KO "$(reason_of "$RUN/b9.out" "$_rc")"
    return
  fi
  _heap=
  [ -z "$_log" ] || _heap=$(logline "$_log" 'heap: internal' | sed 's/^.*heap: //')
  if [ -z "$_heap" ]; then
    report B9 "memory of the updated image" KO "no heap line in the serial log ${_log:-}"
  else
    report B9 "memory of the updated image" OK "$_heap"
  fi
}

step_b10() {
  if ! $YES_FLASH && ! $API_ONLY; then report B10 "factory reset (J4)" skipped "needs --yes-flash"; return; fi
  if ! $FACTORY_RESET; then report B10 "factory reset (J4)" skipped "erases the home: needs --factory-reset"; return; fi
  precheck B10 "factory reset (J4)" api w || return
  account_usable B10 "factory reset (J4)" || return
  refresh_password
  via_saves B10 7 sh "$SELF" --group reset
  finish_group B10 "factory reset (J4)" "$SV_OUT" "$SV_RC"
}

# ---- the gesture steps (R6) ----

HAVE_TTY=false
(exec </dev/tty) 2>/dev/null && HAVE_TTY=true

# The script recipe on the box (spec 041, R7; spec 046): a person at the garage's motion sensor. The
# group checks the garage's own `motion-light` instance: the light goes on, then off after its
# timeout (it installs the signed file given to the session first when the recipe is missing, and
# never uninstalls a recipe an instance uses); this step then reads the serial log of its `saves`
# session for the `scripts:` line (the instance listed, with calls) and the `heap:` line before it,
# whose internal RAM minimum must be at least INTERNAL_MIN_KB.
step_b11() {
  _name="script recipe (041)"
  if ! $GESTURES; then report B11 "$_name" skipped "needs a person: --with-gestures"; return; fi
  if ! $HAVE_TTY && [ "${SOLITE_SESSION_WAVE:-}" != auto ]; then report B11 "$_name" skipped "no terminal"; return; fi
  precheck B11 "$_name" api w || return
  account_usable B11 "$_name" || return
  refresh_password
  via_saves B11 5 sh "$SELF" --group script
  if grep -aq 'box-session: B11.1 .*: skipped (not installed: --recipe-file)' "$SV_OUT" 2>/dev/null; then
    printf 'box-session: B11.6 serial lines: skipped (not installed: --recipe-file)\n' >>"$SV_OUT"
  elif grep -aq 'box-session: B11.2 .*: skipped (the box has no enabled motion-light instance' "$SV_OUT" 2>/dev/null; then
    printf 'box-session: B11.6 serial lines: skipped (no motion-light instance on the box)\n' >>"$SV_OUT"
  elif $API_ONLY; then
    printf 'box-session: B11.6 serial lines: skipped (box only)\n' >>"$SV_OUT"
  elif [ -z "$SV_LOG" ] || [ ! -f "$SV_LOG" ]; then
    printf 'box-session: B11.6 serial lines: KO (no serial log)\n' >>"$SV_OUT"
  else
    _pair=$(awk '/heap: internal/ { h = $0 } /scripts: motion-light [1-9][0-9]* instances/ { print h; print $0 }' "$SV_LOG" | tail -n 2)
    _h=$(printf '%s\n' "$_pair" | sed -n 1p)
    _sl=$(printf '%s\n' "$_pair" | sed -n 2p)
    _min=$(printf '%s' "$_h" | sed -n 's/.*heap: internal [0-9]* KB free (min \([0-9]*\) KB).*/\1/p')
    if [ -z "$_sl" ] || [ -z "$_h" ]; then
      printf 'box-session: B11.6 serial lines: KO (no scripts: line with the script instance in the log, or no heap: line before it)\n' >>"$SV_OUT"
    elif [ -z "$_min" ] || [ "$_min" -lt "$INTERNAL_MIN_KB" ]; then
      printf 'box-session: B11.6 serial lines: KO (internal RAM minimum %s KB, below %s KB)\n' "${_min:-unreadable}" "$INTERNAL_MIN_KB" >>"$SV_OUT"
    else
      printf 'box-session: B11.6 serial lines: OK (%s; %s)\n' "$(printf '%s' "$_sl" | sed 's/^.*scripts: //')" "$(printf '%s' "$_h" | sed 's/^.*heap: //')" >>"$SV_OUT"
    fi
  fi
  finish_group B11 "$_name" "$SV_OUT" "$SV_RC"
}


# $1 id, $2 spec, $3 what the person does (short), $4 what to look at.
gesture() {
  if ! $GESTURES; then report "$1" "gesture ($2)" skipped "needs a person: $3"; return; fi
  if ! $HAVE_TTY; then report "$1" "gesture ($2)" skipped "no terminal"; return; fi
  printf '\nbox-session: %s (spec %s): %s\n  look at: %s\n  seen as expected? [y/n/s=skip] ' "$1" "$2" "$3" "$4" >/dev/tty
  _answer=
  IFS= read -r _answer </dev/tty || _answer=s
  case "$_answer" in
    y | Y) report "$1" "gesture ($2)" OK ;;
    n | N) report "$1" "gesture ($2)" KO "not seen as expected: $4" ;;
    *) report "$1" "gesture ($2)" skipped "by the user" ;;
  esac
}

gestures() {
  gesture H1 021 "unplug the box for 10 s, plug it back" "it comes back, the network resumed (a power cut)"
  gesture H2 021 "power the ZBMINIR2" "it answers an order from the interface"
  gesture H3 026 "by day, with \"Inactive during day\" set, put a hand in front of the motion sensor" "the light does not switch on"
  gesture H4 023 "switch the access point off for 60 s, then on" "the box answers again (the lost link)"
  gesture H5 023 "boot the box with the access point off" "the date is not 1970 once the link is back"
  gesture H6 023 "unplug the relay" "an order answers 502 in the interface"
  gesture H7 029 "open the interface in the browser" "the login screen, /ws live with the token, Discovery's \"Add a device\" opens pairing"
  gesture H8 029 "restart the box by software (the interface's restart, or box.sh wifi again)" "it asks to log in again and keeps the account"
  gesture H9 032 "unplug the box during the minute after an update" "the old image comes back"
  gesture H10 032 "make the box refuse a file in the Firmware update card" "the card shows the error"
  gesture H11 031 "pair a real colour bulb (if you have one)" "its colour orders are applied and read back"
}

# ---- the run ----

BASE_VERSION=$(fw_version)
NEXT_VERSION=$(bump "$BASE_VERSION" 1)

step_p1
step_p2
step_p3
step_p4
step_p5
step_b1
step_b2
step_b3
step_b4
step_b5
step_b6
step_b7
step_b11
step_b8
step_b9
step_b10
gestures

_end="box-session: $N_OK OK, $N_KO KO, $N_SKIP skipped; log: $SESSION_LOG"
[ ! -f "$RUN/password-created" ] || _end="$_end; password: $(cat "$RUN/password-created")"
printf '%s\n' "$_end"
printf '%s\n' "$_end" >>"$SESSION_LOG"
[ "$N_KO" -eq 0 ]
exit $?
