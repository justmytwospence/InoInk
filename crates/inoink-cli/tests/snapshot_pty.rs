//! Exercise login cancellation on a real terminal without contacting a server or opening a browser.
#[test]
fn hidden_login_restores_terminal_on_signals() {
    let script = r#"
import os, pty, select, signal, subprocess, sys, tempfile, termios, time

for sig in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT):
    master, slave = pty.openpty()
    original = termios.tcgetattr(slave)
    env = os.environ.copy()
    for name in ('INOINK_CLIENT_ID', 'INOINK_CLIENT_SECRET', 'INOINK_REDIRECT_URI'):
        env.pop(name, None)
    with tempfile.TemporaryDirectory() as directory:
        child = subprocess.Popen([sys.argv[1], 'snapshot-login', '--auth', os.path.join(directory, 'auth')],
                                 stdin=slave, stdout=slave, stderr=slave, env=env)
        try:
            output = b''
            deadline = time.monotonic() + 10
            while b'Client ID (hidden): ' not in output and time.monotonic() < deadline:
                readable, _, _ = select.select([master], [], [], 0.1)
                if readable:
                    output += os.read(master, 4096)
                if child.poll() is not None:
                    raise AssertionError('login exited before prompt: ' + repr(output))
            assert b'Client ID (hidden): ' in output, repr(output)
            assert not termios.tcgetattr(slave)[3] & termios.ECHO
            os.write(master, b'dummy-private-input')
            os.kill(child.pid, sig)
            assert child.wait(timeout=10) == 128 + sig
            assert termios.tcgetattr(slave) == original
            assert not os.path.exists(os.path.join(directory, 'auth', 'credentials.json'))
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            os.close(master)
            os.close(slave)
"#;
    let status = std::process::Command::new("python3")
        .args(["-c", script, env!("CARGO_BIN_EXE_inoink-cli")])
        .status()
        .expect("python3 is required for the PTY test");
    assert!(status.success());
}

#[test]
fn callback_wait_can_be_cancelled_after_hidden_prompts() {
    let script = r#"
import os, pathlib, pty, select, shutil, signal, socket, subprocess, sys, tempfile, termios, time

cli = os.path.abspath(sys.argv[1])
root = pathlib.Path(sys.argv[2]).parents[1]

def executable(path, body):
    path.write_text('#!/bin/sh\nset -eu\n' + body + '\n')
    path.chmod(0o755)

def launcher_fixture(directory, bin):
    # Exercise the real launcher's signal forwarding/reaping with the real CLI.
    # All native dependency/build commands are stubbed; no network or display.
    repo = directory / 'repo with spaces'
    (repo / 'scripts').mkdir(parents=True)
    (repo / 'build/inputs').mkdir(parents=True)
    (repo / 'mupdf_wrapper').mkdir()
    shutil.copy(root / 'scripts/preview-macos.sh', repo / 'scripts')
    (repo / 'build/inputs.lock.json').write_text('{}')
    (repo / 'build/inputs/mupdf-1.27.0-source.tar.gz').write_text('fake archive')
    executable(repo / 'scripts/version.sh', 'echo dev')
    executable(repo / 'mupdf_wrapper/build.sh',
               'mkdir -p ../target/mupdf_wrapper/Darwin; touch ../target/mupdf_wrapper/Darwin/libmupdf_wrapper.a')
    stubs = {
        'uname': 'case $1 in -s) echo Darwin;; -m) echo arm64;; esac',
        'brew': 'case $1 in --prefix) echo "$TEST_REPO/brew/${2:-base}";; list) echo "$3 1.0";; esac',
        'xcrun': 'echo fake-clang',
        'shasum': 'echo "fakehash  file"',
        'jq': 'echo fakehash',
        'rustc': 'echo "host: fake"',
        'pkg-config': 'echo fake',
        'sysctl': 'echo 2',
        'tar': 'while [ "$#" -gt 0 ]; do if [ "$1" = -C ]; then mkdir -p "$2/build/release" "$2/include"; fi; shift; done',
        'make': 'mkdir -p build/release; touch build/release/libmupdf.a build/release/libmupdf-third.a',
        'cargo': '''bin="$CARGO_TARGET_DIR/aarch64-apple-darwin/debug"
mkdir -p "$bin"
ln -s "$TEST_CLI" "$bin/inoink-cli"
printf '#!/bin/sh\\nexit 99\\n' > "$bin/inoink-emulator"
chmod +x "$bin/inoink-emulator"''',
    }
    for name, body in stubs.items():
        executable(bin / name, body)
    return repo

for launcher in (False, True):
    signals = (signal.SIGINT, signal.SIGTERM) if launcher else (
        signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT)
    for sig in signals:
        with tempfile.TemporaryDirectory() as temporary:
            directory = pathlib.Path(temporary)
            bin = directory / 'bin'
            bin.mkdir()
            # Never invoke the real browser or record the consent URL.
            for name in ('open', 'xdg-open'):
                executable(bin / name, 'echo "$PPID" > "$TEST_LOGIN_PID"')
            env = os.environ.copy()
            for name in ('INOINK_CLIENT_ID', 'INOINK_CLIENT_SECRET', 'INOINK_REDIRECT_URI'):
                env.pop(name, None)
            env.update(PATH=str(bin) + os.pathsep + env['PATH'],
                       TEST_CLI=cli, TEST_LOGIN_PID=str(directory / 'login-pid'))
            if launcher:
                repo = launcher_fixture(directory, bin)
                env['TEST_REPO'] = str(repo)
                command = ['sh', str(repo / 'scripts/preview-macos.sh'), '--inoreader']
                base = repo / 'target/macos-preview'
                auth = base / 'inoreader/auth'
            else:
                auth = directory / 'auth'
                command = [cli, 'snapshot-login', '--auth', str(auth)]
            master, slave = pty.openpty()
            original = termios.tcgetattr(slave)
            # Reserve a free loopback port until the CLI is ready to bind it.
            reservation = socket.socket()
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
            child = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, env=env)
            login_pid = None
            output = b''
            def wait_for(marker):
                global output
                deadline = time.monotonic() + 20
                while marker not in output and time.monotonic() < deadline:
                    readable, _, _ = select.select([master], [], [], 0.1)
                    if readable:
                        output += os.read(master, 4096)
                    assert child.poll() is None, ('early exit', launcher, sig, output)
                assert marker in output, ('missing output', launcher, sig, output)
            try:
                for prompt, value in (
                    (b'Client ID (hidden): ', b'dummy-client'),
                    (b'Client secret (hidden): ', b'dummy-secret'),
                    (b'Registered callback URI (hidden): ', f'http://127.0.0.1:{port}/callback'.encode()),
                ):
                    wait_for(prompt)
                    assert not termios.tcgetattr(slave)[3] & termios.ECHO
                    if prompt.startswith(b'Registered'):
                        reservation.close()
                    os.write(master, value + b'\n')
                wait_for(b'Waiting for browser authorization.')
                login_pid = int((directory / 'login-pid').read_text())
                assert termios.tcgetattr(slave) == original
                assert b'dummy-client' not in output and b'dummy-secret' not in output
                # No prompt is active now. Cancellation must not restore a stale
                # saved terminal state over changes made after the last prompt.
                changed = termios.tcgetattr(slave)
                changed[0] ^= termios.IXON
                termios.tcsetattr(slave, termios.TCSANOW, changed)
                os.kill(child.pid, sig)
                assert child.wait(timeout=10) == 128 + sig, (launcher, sig, output)
                assert termios.tcgetattr(slave) == changed
                if launcher:
                    try:
                        os.kill(login_pid, 0)
                    except ProcessLookupError:
                        pass
                    else:
                        raise AssertionError('launcher left login running')
                    assert not (base / 'run.lock').exists()
                    assert not (base / 'inoreader/state/data').exists()
                    assert not list((base / 'inoreader').glob('staging.*'))
                assert not (auth / 'credentials.json').exists()
                assert not (auth / 'snapshot-auth.json').exists()
            finally:
                # On regression, don't leave a callback listener behind.
                if login_pid is None and (directory / 'login-pid').exists():
                    login_pid = int((directory / 'login-pid').read_text())
                if login_pid is not None:
                    try:
                        os.kill(login_pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                if child.poll() is None:
                    child.kill()
                    child.wait()
                reservation.close()
                termios.tcsetattr(slave, termios.TCSANOW, original)
                os.close(master)
                os.close(slave)
"#;
    let status = std::process::Command::new("python3")
        .args(["-c", script, env!("CARGO_BIN_EXE_inoink-cli"), env!("CARGO_MANIFEST_DIR")])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("PYTHONDONTWRITEBYTECODE", "1")
        .status()
        .expect("python3 is required for the PTY test");
    assert!(status.success());
}
