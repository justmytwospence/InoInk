#!/bin/sh
# Portable orchestration tests; no network, real builds, or display.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd -P)
WORK=$(mktemp -d)
WORK=$(CDPATH= cd -- "$WORK" && pwd -P)
controller= signal_child=
cleanup() {
 # A failed lifecycle assertion must not leave a blocked fake emulator behind.
 [ -z "$controller" ] || kill -s TERM "$controller" 2>/dev/null || :
 [ -z "$signal_child" ] || kill -s KILL "$signal_child" 2>/dev/null || :
 rm -rf "$WORK"
}
trap cleanup EXIT
repo="$WORK/repo with spaces"; bin="$WORK/bin"; log="$WORK/log"
mkdir -p "$repo/scripts" "$repo/build/inputs" "$repo/mupdf_wrapper" "$bin"
cp "$ROOT/scripts/preview-macos.sh" "$repo/scripts/"
cp "$ROOT/build/inputs.lock.json" "$repo/build/"
printf 'fake archive\n' > "$repo/build/inputs/mupdf-1.27.0-source.tar.gz"
printf '#!/bin/sh\necho dev\n' > "$repo/scripts/version.sh"
printf '#!/bin/sh\nmkdir -p ../target/mupdf_wrapper/Darwin; touch ../target/mupdf_wrapper/Darwin/libmupdf_wrapper.a\n' > "$repo/mupdf_wrapper/build.sh"
chmod +x "$repo/scripts/version.sh"
export TEST_LOG="$log" TEST_REPO="$repo" PATH="$bin:$PATH"
cat > "$bin/uname" <<'EOF'
#!/bin/sh
case $1 in -s) echo "${FAKE_OS:-Darwin}";; -m) echo "${FAKE_ARCH:-arm64}";; esac
EOF
cat > "$bin/brew" <<'EOF'
#!/bin/sh
echo "brew $*" >> "$TEST_LOG"
case $1 in
 --prefix) echo "$TEST_REPO/brew/${2:-base}";;
 list) if [ "${3:-}" = "${MISSING:-none}" ] && [ ! -e "$TEST_REPO/installed" ]; then exit 1; fi; echo "$3 1.0";;
 install) touch "$TEST_REPO/installed";;
esac
EOF
cat > "$bin/xcrun" <<'EOF'
#!/bin/sh
if [ "$1" = --find ]; then echo /usr/bin/clang; else echo fake-clang; fi
EOF
cat > "$bin/shasum" <<'EOF'
#!/bin/sh
case $3 in *mupdf-1.27.0-source.tar.gz) echo "${BAD_HASH:-ae2442416de499182d37a526c6fa2bacc7a3bed5a888d113ca04844484dfe7c6}  file";; *) echo 'script-hash  file';; esac
EOF
cat > "$bin/tar" <<'EOF'
#!/bin/sh
while [ "$#" -gt 0 ]; do if [ "$1" = -C ]; then mkdir -p "$2/build/release" "$2/include"; fi; shift; done
EOF
cat > "$bin/make" <<'EOF'
#!/bin/sh
echo "make $*" >> "$TEST_LOG"
mkdir -p build/release; touch build/release/libmupdf.a build/release/libmupdf-third.a
EOF
cat > "$bin/pkg-config" <<'EOF'
#!/bin/sh
echo fake
EOF
cat > "$bin/sysctl" <<'EOF'
#!/bin/sh
echo 2
EOF
cat > "$bin/mv" <<'EOF'
#!/bin/sh
case ${FAIL_PUBLISH:-0}:$1 in 1:*/staging.*/data) exit 7 ;; esac
/bin/mv "$@" || exit $?
case ${CANCEL_PUBLISH:-}:$1 in
 backup:*/state/data|install:*/staging.*/data)
  kill -s TERM "$(cat "$TEST_REPO/target/macos-preview/run.lock/pid")" ;;
esac
EOF
cat > "$bin/cargo" <<'EOF'
#!/bin/sh
echo "cargo $* target=$CARGO_TARGET_DIR flags=$CARGO_ENCODED_RUSTFLAGS" >> "$TEST_LOG"
bin="$CARGO_TARGET_DIR/aarch64-apple-darwin/debug"; mkdir -p "$bin"
cat > "$bin/inoink-cli" <<'INNER'
#!/bin/sh
echo "$*" >> "$TEST_LOG"
check_stdin() {
 [ "${CHECK_STDIN:-0}" = 1 ] || return 0
 [ -t 0 ] || exit 81
 printf 'stdin-ready %s\n' "$1"
 IFS= read -r input || exit 82
 [ "$input" = "dummy-$1" ] || exit 83
}
wait_for_term() {
 trap 'printf "term %s\n" "$$" >&3; IFS= read -r command <&4; exit 42' TERM
 printf 'ready %s\n' "$$" >&3
 while :; do IFS= read -r command <&4 || exit 92; done
}
case $1 in
 fixtures)
  [ "${FIXTURE_FAIL:-0}" = 0 ] || exit 4
  mkdir -p "$3"; echo fixture > "$3/db" ;;
 snapshot-login)
  check_stdin login
  [ "${LOGIN_WAIT:-0}" = 0 ] || wait_for_term
  if [ "${EXPECT_LOGIN_ENV:-0}" = 1 ]; then
   [ "${INOINK_CLIENT_ID:-}" = id ] && [ "${INOINK_CLIENT_SECRET:-}" = secret ] && [ "${INOINK_REDIRECT_URI:-}" = uri ] || exit 8
  fi
  [ "${LOGIN_FAIL:-0}" = 0 ] || exit 5
  mkdir -p "$3"; echo credentials > "$3/credentials.json" ;;
 snapshot)
  check_stdin snapshot
  [ "${SNAPSHOT_WAIT:-0}" = 0 ] || wait_for_term
  [ "${SNAPSHOT_FAIL:-0}" = 0 ] || exit 6
  mkdir -p "$5"; echo fresh > "$5/db"
  [ "${NO_MARKER:-0}" = 1 ] || touch "$5/preview-sandbox.json" ;;
 *) exit 99 ;;
esac
INNER
cat > "$bin/inoink-emulator" <<'INNER'
#!/bin/sh
echo "emulator $* PRODUCT=$PRODUCT MODEL_NUMBER=$MODEL_NUMBER SETTINGS=$INOINK_SETTINGS SDL=${SDL_VIDEODRIVER-unset} SECRETS=${INOINK_CLIENT_ID-unset}/${INOINK_CLIENT_SECRET-unset}/${INOINK_REDIRECT_URI-unset}" >> "$TEST_LOG"
lock="$TEST_REPO/target/macos-preview/run.lock"
if [ "${CHECK_STDIN:-0}" = 1 ]; then
 [ -t 0 ] || exit 81
 printf 'stdin-ready emulator\n'
 IFS= read -r input || exit 82
 [ "$input" = dummy-emulator ] || exit 83
fi
case ${CHILD_LOCK:-} in
 replace)
  mv "$lock" "$lock.original"
  mkdir "$lock"; echo "$$" > "$lock/pid"; echo keep > "$lock/keep"
  ;;
 missing-pid) rm "$lock/pid"; echo keep > "$lock/keep" ;;
 extra-file) echo keep > "$lock/keep" ;;
esac
if [ "${CHILD_WAIT:-0}" = 1 ]; then
 on_term() {
  trap '' TERM
  printf 'term %s\n' "$$" >&3
  # Hold shutdown open until the test has checked the child and lock.
  IFS= read -r command <&4
  [ "$command" = exit ] || exit 91
  exit 23
 }
 trap on_term TERM
 printf 'ready %s\n' "$$" >&3
 while :; do IFS= read -r command <&4 || exit 92; done
fi
exit "${CHILD_EXIT:-0}"
INNER
chmod +x "$bin/inoink-cli" "$bin/inoink-emulator"
EOF
chmod +x "$bin"/*
run() { sh "$repo/scripts/preview-macos.sh" "$@" > "$WORK/out" 2>&1; }
fail() { echo "FAIL: $*"; [ ! -f "$WORK/out" ] || tail -20 "$WORK/out"; [ ! -f "$log" ] || tail -5 "$log"; exit 1; }
run --help || fail help
FAKE_OS=Linux run --build-only && fail 'unsupported OS' || :
unset FAKE_OS
FAKE_ARCH=x86_64 run --build-only && fail 'unsupported architecture' || :
unset FAKE_ARCH
for args in '--scale' '--scale nan' '--scale 0' '--scale 2.1' '--wat' '--reset --build-only' '--refresh' '--inoreader --refresh --build-only' '--inoreader --reset'; do
 # Intentional split of fixed test cases.
 # shellcheck disable=SC2086
 status=0; run $args || status=$?; [ "$status" = 2 ] || fail "invalid arguments: $args"
done
[ ! -e "$log" ] || fail 'invalid arguments ran commands'
BAD_HASH=bad run --build-only && fail 'checksum' || :
unset BAD_HASH
! grep -q '^make\|^cargo' "$log" || fail 'checksum allowed build'
: > "$log"
MISSING=gumbo-parser run --build-only || fail 'first build'
unset MISSING
grep -q '^brew install gumbo-parser$' "$log" || fail 'missing formula install'
! grep -q '^fixtures\|^emulator' "$log" || fail 'build-only launched'
[ ! -d "$repo/target/macos-preview/state" ] || fail 'build-only created state'
: > "$log"; run --build-only || fail 'cached build'
! grep -q '^make' "$log" || fail 'cached build rebuilt native'
grep -q '^cargo' "$log" || fail 'cached build skipped cargo'
rm "$repo/target/macos-preview/native/lib/libmupdf.a"
: > "$log"; run --build-only || fail 'missing archive rebuild'
grep -q '^make' "$log" || fail 'missing archive not rebuilt'
: > "$log"
CARGO_TARGET_DIR=/bad INOINK_SETTINGS=/bad SDL_VIDEODRIVER=dummy run --scale 0.4 || fail launch
unset CARGO_TARGET_DIR INOINK_SETTINGS SDL_VIDEODRIVER
state="$repo/target/macos-preview/state"
grep -q '^fixtures ' "$log" || fail fixtures
grep -Fq "--scale 0.4 --screenshots $state/screenshots PRODUCT=monza MODEL_NUMBER=390 SETTINGS=$state/Settings.toml SDL=unset" "$log" 2>/dev/null || grep -Fq -- "--scale 0.4 --screenshots $state/screenshots PRODUCT=monza MODEL_NUMBER=390 SETTINGS=$state/Settings.toml SDL=unset" "$log" || fail isolation
! grep '^emulator' "$log" | grep -q -- '--online\|--scripted-tour' || fail offline
echo keep > "$state/Settings.toml"; echo cache > "$repo/target/macos-preview/native/keep"
: > "$log"; run || fail relaunch
! grep -q '^fixtures' "$log" || fail 'regenerated fixtures'
[ "$(cat "$state/Settings.toml")" = keep ] || fail 'lost settings'
run --reset || fail reset
[ ! -e "$state/Settings.toml" ] && [ -f "$repo/target/macos-preview/native/keep" ] || fail 'reset scope'
rm -rf "$state/data"
FIXTURE_FAIL=1 run && fail 'fixture failure' || :
unset FIXTURE_FAIL
[ ! -e "$state/data" ] || fail 'fixture failure left data'
CHILD_EXIT=7 run && fail 'child failure' || status=$?
unset CHILD_EXIT
[ "$status" = 7 ] || fail 'child status'
lock="$repo/target/macos-preview/run.lock"
[ ! -e "$lock" ] || fail 'lock cleanup'
mkdir "$lock"; echo $$ > "$lock/pid"
run --build-only && fail 'live lock' || :
[ -d "$lock" ] || fail 'removed live lock'
[ "$(cat "$lock/pid")" = "$$" ] || fail 'changed live lock owner'
for pid in 99999999 invalid ''; do
 if [ -n "$pid" ]; then echo "$pid" > "$lock/pid"; else rm "$lock/pid"; fi
 : > "$log"
 run --build-only && fail 'stale or incomplete lock accepted' || :
 grep -q 'refusing automatic removal' "$WORK/out" || fail 'missing lock refusal'
 grep -Fq "$lock" "$WORK/out" || fail 'missing manual recovery path'
 [ -d "$lock" ] || fail 'removed stale or incomplete lock'
 if [ -n "$pid" ]; then
  [ "$(cat "$lock/pid")" = "$pid" ] || fail 'changed refused lock owner'
 else
  [ ! -e "$lock/pid" ] || fail 'created PID for incomplete lock'
 fi
 [ ! -s "$log" ] || fail 'refused lock ran build commands'
done
rm -r "$lock"
for mode in replace missing-pid extra-file; do
 CHILD_LOCK=$mode run || fail "lock mutation: $mode"
 unset CHILD_LOCK
 [ "$(cat "$lock/keep")" = keep ] || fail "deleted unowned lock contents: $mode"
 case $mode in
  replace)
   [ -f "$lock/pid" ] || fail 'removed replacement owner'
   [ "$(cat "$lock/pid")" != "$(cat "$lock.original/pid")" ] || fail 'replacement test reused owner'
   rm -r "$lock.original"
   ;;
  *) [ ! -e "$lock/pid" ] || fail "left owned PID: $mode" ;;
 esac
 rm -r "$lock"
done

# Keep the launcher in the foreground: an asynchronous shell itself inherits INT
# ignored. Only its emulator should inherit that disposition. A controller uses
# FIFOs for readiness and shutdown acknowledgment, never timing-based polling.
mkfifo "$WORK/events" "$WORK/commands"
exec 3<> "$WORK/events" 4<> "$WORK/commands"
for signal in INT TERM; do
 (
  launcher=
  # Unblock shutdown even if a controller assertion fails.
  trap 'printf "exit\n" >&4' EXIT
  IFS=' ' read -r event pid <&3
  [ "$event" = ready ] || fail 'child readiness'
  echo "$pid" > "$WORK/signal-child"
  launcher=$(cat "$lock/pid")
  [ "$launcher" != "$pid" ] || fail 'launcher PID is child PID'
  kill -0 "$pid" || fail 'child not running at readiness'
  kill -s "$signal" "$launcher" || fail "send $signal"
  IFS=' ' read -r event stopped_pid <&3
  [ "$event" = term ] && [ "$stopped_pid" = "$pid" ] || fail "$signal not forwarded as TERM"
  kill -0 "$pid" || fail 'child exited before shutdown acknowledgment'
  [ "$(cat "$lock/pid")" = "$launcher" ] || fail 'lock released before child exit'
  sh "$repo/scripts/preview-macos.sh" --build-only > "$WORK/contender-out" 2>&1 && fail 'concurrent launcher accepted during shutdown'
  grep -q 'preview already running' "$WORK/contender-out" || fail 'shutdown lock not live'
 ) &
 controller=$!
 status=0; CHILD_WAIT=1 run || status=$?
 unset CHILD_WAIT
 signal_child=$(cat "$WORK/signal-child")
 wait "$controller" || fail "$signal controller"
 controller=
 [ "$status" = 23 ] || fail "$signal child exit status: $status"
 ! kill -0 "$signal_child" 2>/dev/null || fail "$signal child not reaped"
 signal_child=
 [ ! -e "$lock" ] || fail "$signal lock not released after child exit"
done
exec 3>&- 4>&-

real="$repo/target/macos-preview/inoreader"
: > "$log"; run --inoreader --build-only || fail 'real build only'
[ ! -e "$real" ] || fail 'real build only created private data'
: > "$log"
LOGIN_FAIL=1 run --inoreader && fail 'login failure accepted' || :
unset LOGIN_FAIL
! grep -q '^snapshot \|^emulator ' "$log" || fail 'login failure continued'
: > "$log"
( export EXPECT_LOGIN_ENV=1 INOINK_CLIENT_ID=id INOINK_CLIENT_SECRET=secret INOINK_REDIRECT_URI=uri; run --inoreader ) || fail 'first snapshot'
[ "$(grep -c '^snapshot-login ' "$log")" = 1 ] && [ "$(grep -c '^snapshot ' "$log")" = 1 ] || fail 'first snapshot commands'
grep '^emulator ' "$log" | grep -q 'SECRETS=unset/unset/unset' || fail 'emulator inherited secrets'
! grep '^emulator ' "$log" | grep -q -- '--online\|/auth' || fail 'real emulator online or received auth path'
[ -f "$real/state/data/preview-sandbox.json" ] || fail 'snapshot marker'
[ "$(ls -ld "$real/state" | cut -c1-10)" = drwx------ ] || fail 'private state permissions'
echo local > "$real/state/data/db"; echo settings > "$real/state/Settings.toml"
rm -rf "$real/auth"
: > "$log"; run --inoreader || fail 'offline reuse without credentials'
! grep -q '^snapshot' "$log" || fail 'reuse contacted auth or network'
[ "$(cat "$real/state/data/db")" = local ] || fail 'reuse lost local changes'
run --inoreader --refresh || fail 'refresh'
[ "$(cat "$real/state/data/db")" = fresh ] && [ "$(cat "$real/state/Settings.toml")" = settings ] || fail 'refresh data/settings'
[ -f "$real/auth/credentials.json" ] && [ -f "$state/data/db" ] && [ -f "$repo/target/macos-preview/native/keep" ] || fail 'refresh isolation'
echo local > "$real/state/data/db"
for mode in SNAPSHOT_FAIL NO_MARKER FAIL_PUBLISH CANCEL_BACKUP CANCEL_INSTALL; do
 : > "$log"
 case $mode in
  SNAPSHOT_FAIL) SNAPSHOT_FAIL=1 run --inoreader --refresh && fail 'download failure accepted' || : ;;
  NO_MARKER) NO_MARKER=1 run --inoreader --refresh && fail 'missing downloaded marker accepted' || : ;;
  FAIL_PUBLISH) FAIL_PUBLISH=1 run --inoreader --refresh && fail 'publication failure accepted' || : ;;
  CANCEL_BACKUP|CANCEL_INSTALL)
   case $mode in CANCEL_BACKUP) point=backup ;; CANCEL_INSTALL) point=install ;; esac
   status=0; CANCEL_PUBLISH=$point run --inoreader --refresh || status=$?
   [ "$status" = 143 ] || fail "publication cancellation status: $status" ;;
 esac
 unset SNAPSHOT_FAIL NO_MARKER FAIL_PUBLISH CANCEL_PUBLISH
 [ "$(cat "$real/state/data/db")" = local ] || fail 'failure lost previous data'
 ! grep -q '^emulator ' "$log" || fail 'failure launched emulator'
 [ -z "$(find "$real" -name 'staging.*' -print)" ] || fail 'failure left staging'
done
rm "$real/state/data/preview-sandbox.json"
: > "$log"; run --inoreader && fail 'unmarked state accepted' || :
! grep -q '^snapshot\|^emulator' "$log" || fail 'unmarked state opened'
touch "$real/state/data/preview-sandbox.json"
run --reset || fail 'sample reset with real data'
[ "$(cat "$real/state/data/db")" = local ] && [ -f "$real/auth/credentials.json" ] || fail 'sample reset removed real data'

# Interrupt the downloader and verify the lock and old snapshot survive until reaped.
mkfifo "$WORK/events2" "$WORK/commands2"
exec 3<> "$WORK/events2" 4<> "$WORK/commands2"
(
 trap 'printf "exit\n" >&4' EXIT
 IFS=' ' read -r event pid <&3
 [ "$event" = ready ] || fail 'download readiness'
 echo "$pid" > "$WORK/signal-child"
 launcher=$(cat "$lock/pid")
 kill -s INT "$launcher"
 IFS=' ' read -r event stopped_pid <&3
 [ "$event" = term ] && [ "$stopped_pid" = "$pid" ] || fail 'download signal forwarding'
 [ "$(cat "$lock/pid")" = "$launcher" ] || fail 'download lock released early'
 [ "$(cat "$real/state/data/db")" = local ] || fail 'download changed old data'
) &
controller=$!
status=0; SNAPSHOT_WAIT=1 run --inoreader --refresh || status=$?
signal_child=$(cat "$WORK/signal-child")
wait "$controller" || fail 'download signal controller'; controller=
[ "$status" = 130 ] || fail "download interruption status: $status"
! kill -0 "$signal_child" 2>/dev/null || fail 'downloader not reaped'; signal_child=
[ ! -e "$lock" ] && [ -z "$(find "$real" -name 'staging.*' -print)" ] || fail 'interruption cleanup'
[ "$(cat "$real/state/data/db")" = local ] || fail 'interruption lost data'
exec 3>&- 4>&-

# Instrument only the copied launcher: hold the exact spawn-to-$! gap open.
# No production test hooks or sleeps. Builtins here leave $! unchanged.
python3 - "$repo/scripts/preview-macos.sh" "$repo/scripts/preview-gap.sh" <<'PY'
import pathlib
import sys
source = pathlib.Path(sys.argv[1]).read_text()
needle = ' child=$!\n'
assert source.count(needle) == 1
hook = ''' if [ "${STARTUP_GAP:-0}" = 1 ]; then
  printf 'gap\\n' >&3
  while [ "$interrupted" = 0 ]; do IFS= read -r command <&5 || :; done
 fi
'''
pathlib.Path(sys.argv[2]).write_text(source.replace(needle, hook + needle))
PY
mkfifo "$WORK/gap-events" "$WORK/gap-commands" "$WORK/gap-release"
exec 3<> "$WORK/gap-events" 4<> "$WORK/gap-commands" 5<> "$WORK/gap-release"
for mode in login snapshot emulator; do
 for signal in INT TERM; do
  case $mode in
   login) rm -rf "$real/auth"; args='--inoreader --refresh'; LOGIN_WAIT=1; export LOGIN_WAIT ;;
   snapshot) mkdir -p "$real/auth"; touch "$real/auth/credentials.json"; args='--inoreader --refresh'; SNAPSHOT_WAIT=1; export SNAPSHOT_WAIT ;;
   emulator) args=; CHILD_WAIT=1; export CHILD_WAIT ;;
  esac
  (
   trap 'printf "exit\n" >&4' EXIT
   ready=0 gap=0 pid=
   while [ "$ready:$gap" != 1:1 ]; do
    IFS=' ' read -r event value <&3
    case $event in
     ready) ready=1; pid=$value ;;
     gap) gap=1 ;;
     *) fail "unexpected startup event: $event" ;;
    esac
   done
   echo "$pid" > "$WORK/signal-child"
   launcher=$(cat "$lock/pid")
   kill -s "$signal" "$launcher" || fail 'startup signal'
   # Release read even on shells that restart it after running a trap.
   printf 'continue\n' >&5
   IFS=' ' read -r event stopped_pid <&3
   [ "$event" = term ] && [ "$stopped_pid" = "$pid" ] || fail 'startup gap did not forward TERM'
   kill -0 "$pid" || fail 'startup child not held for reaping'
   [ "$(cat "$lock/pid")" = "$launcher" ] || fail 'startup lock released early'
   [ "$(cat "$real/state/data/db")" = local ] || fail 'startup changed old snapshot'
   sh "$repo/scripts/preview-macos.sh" --build-only > "$WORK/contender-out" 2>&1 && fail 'startup gap allowed concurrent launch'
   grep -q 'preview already running' "$WORK/contender-out" || fail 'startup lock not live'
  ) &
  controller=$!
  status=0
  # Intentional split of fixed arguments.
  # shellcheck disable=SC2086
  STARTUP_GAP=1 sh "$repo/scripts/preview-gap.sh" $args > "$WORK/out" 2>&1 || status=$?
  signal_child=$(cat "$WORK/signal-child")
  wait "$controller" || fail "startup controller: $mode $signal"; controller=
  case $mode:$signal in emulator:*) expected=23 ;; *:INT) expected=130 ;; *:TERM) expected=143 ;; esac
  [ "$status" = "$expected" ] || fail "startup status: $mode $signal $status"
  ! kill -0 "$signal_child" 2>/dev/null || fail 'startup child not reaped'; signal_child=
  [ ! -e "$lock" ] || fail 'startup lock not cleaned'
  [ -z "$(find "$real" -name 'staging.*' -print)" ] || fail 'startup staging not cleaned'
  unset LOGIN_WAIT SNAPSHOT_WAIT CHILD_WAIT
 done
done
exec 3>&- 4>&- 5>&-

# Exercise a real terminal stdin, including first-use login, downloader, and
# emulator, then the ordinary sample path. Only fixed dummy input is sent.
python3 - "$repo" <<'PY'
import errno
import os
import pathlib
import pty
import select
import signal
import sys
import time

repo = pathlib.Path(sys.argv[1])
real = repo / 'target/macos-preview/inoreader'
(real / 'auth/credentials.json').unlink(missing_ok=True)
for args, expected in [(['--inoreader', '--refresh'], ['login', 'snapshot', 'emulator']),
                       ([], ['emulator'])]:
    pid, terminal = pty.fork()
    if pid == 0:
        os.environ['CHECK_STDIN'] = '1'
        os.execvp('sh', ['sh', str(repo / 'scripts/preview-macos.sh'), *args])
    transcript = b''
    pending = list(expected)
    deadline = time.monotonic() + 30
    reaped = False
    try:
        while True:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f'PTY timeout: {transcript!r}'
            readable, _, _ = select.select([terminal], [], [], remaining)
            assert readable, f'PTY timeout: {transcript!r}'
            try:
                data = os.read(terminal, 65536)
            except OSError as exc:
                if exc.errno != errno.EIO:
                    raise
                data = b''
            if not data:
                break
            transcript += data
            if pending and f'stdin-ready {pending[0]}\r\n'.encode() in transcript:
                stage = pending.pop(0)
                os.write(terminal, f'dummy-{stage}\n'.encode())
        _, status = os.waitpid(pid, 0)
        reaped = True
        assert os.waitstatus_to_exitcode(status) == 0, transcript.decode(errors='replace')
        assert not pending, f'missing stdin probes: {pending}; {transcript!r}'
        assert not (repo / 'target/macos-preview/run.lock').exists()
    finally:
        if not reaped:
            os.killpg(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(terminal)
PY
echo 'preview-macos tests passed'
