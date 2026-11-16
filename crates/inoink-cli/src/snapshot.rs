//! Private, read-only authorization and offline snapshot download.
use std::io::{BufRead, IsTerminal, Read, Write};
use std::net::TcpListener;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use inoink::api::auth::{authorize_url, exchange_code, parse_callback, AppConfig, TokenStore, Tokens};
use inoink::api::{ApiClient, SystemClock, UreqTransport};
use inoink::content::images::UreqFetcher;
use inoink::credentials::{Credentials, FileCredentials};
use inoink::error::{Error, Result};
use inoink::snapshot::{self, ReadOnlyTokens, ReadOnlyTransport};

fn invalid(message: &str) -> Error {
    Error::InvalidRequest(message.into())
}
fn safe_failure() -> Error {
    Error::Data("Snapshot operation failed; check authorization, network and available storage.".into())
}

fn arguments(args: &[String], output: bool) -> Result<(PathBuf, Option<PathBuf>)> {
    let mut auth = None;
    let mut out = None;
    let mut it = args.iter();
    while let Some(key) = it.next() {
        let slot = match key.as_str() {
            "--auth" => &mut auth,
            "--output" if output => &mut out,
            _ => return Err(invalid("unexpected snapshot argument")),
        };
        if slot.is_some() {
            return Err(invalid("duplicate snapshot argument"));
        }
        let value = it.next().filter(|v| !v.is_empty() && !v.starts_with("--")).ok_or_else(|| invalid("snapshot argument needs a path"))?;
        *slot = Some(PathBuf::from(value));
    }
    let auth = auth.ok_or_else(|| invalid("--auth is required"))?;
    if output && out.is_none() {
        return Err(invalid("--output is required"));
    }
    Ok((auth, out))
}

// Resolve existing components, including symlinks, before appending nonexistent components.
fn resolved(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir()?.join(path) };
    let mut result = PathBuf::from("/");
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            Component::Normal(part) => {
                result.push(part);
                match std::fs::symlink_metadata(&result) {
                    Ok(_) => result = std::fs::canonicalize(&result)?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                }
            }
            Component::Prefix(_) => return Err(invalid("unsupported path")),
        }
    }
    Ok(result)
}

fn separate(auth: &Path, out: &Path) -> Result<()> {
    let a = resolved(auth)?;
    let b = resolved(out)?;
    if a.starts_with(&b) || b.starts_with(&a) {
        return Err(invalid("auth and output paths must not overlap"));
    }
    Ok(())
}

fn exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

fn private_dir(path: &Path, create: bool) -> Result<()> {
    if create && !exists(path)? {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    }
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.permissions().mode() & 0o7777 != 0o700 {
        return Err(invalid("auth directory must be private (mode 0700) and not a symlink"));
    }
    Ok(())
}

fn private_file(path: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.is_file() || meta.permissions().mode() & 0o7777 != 0o600 {
        return Err(invalid("credentials must be a private regular file (mode 0600)"));
    }
    Ok(())
}

fn restore_echo(state: &str) {
    let _ = Command::new("stty").arg(state).stdin(Stdio::inherit()).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

// Signal-hook does not restore default dispositions when handlers are removed.
// Install once for the entire login, including browser/callback and network waits.
struct LoginSignals {
    echo: Arc<Mutex<Option<String>>>,
    handle: signal_hook::iterator::Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl LoginSignals {
    fn new() -> Result<Self> {
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
            signal_hook::consts::SIGQUIT,
        ])?;
        let handle = signals.handle();
        let echo = Arc::new(Mutex::new(None::<String>));
        let signal_echo = echo.clone();
        let thread = std::thread::spawn(move || {
            if let Some(signal) = signals.forever().next() {
                // Hold the lock through exit: a prompt must not disable echo after
                // cancellation has restored it. Outside a prompt, leave stty alone.
                let mut state = signal_echo.lock().unwrap();
                if let Some(state) = state.take() {
                    restore_echo(&state);
                }
                std::process::exit(128 + signal);
            }
        });
        Ok(Self { echo, handle, thread: Some(thread) })
    }
}
impl Drop for LoginSignals {
    fn drop(&mut self) {
        self.handle.close();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct EchoGuard<'a>(&'a Mutex<Option<String>>);
impl Drop for EchoGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.0.lock().unwrap();
        if let Some(state) = state.take() {
            restore_echo(&state);
        }
    }
}
fn hidden(prompt: &str, signals: &mut LoginSignals) -> Result<String> {
    if !std::io::stdin().is_terminal() {
        return Err(invalid("required input is missing; use an interactive terminal or environment variables"));
    }
    let state = Command::new("stty").arg("-g").stdin(Stdio::inherit()).stderr(Stdio::null()).output()?;
    if !state.status.success() {
        return Err(invalid("could not secure terminal input"));
    }
    let state = String::from_utf8(state.stdout).map_err(|_| invalid("could not secure terminal input"))?;
    let state = state.trim().to_string();
    // The mutable login borrow prevents overlapping prompts. Serialize both stty
    // transitions with cancellation so restoration cannot precede disabling echo.
    let mut echo = signals.echo.lock().unwrap();
    *echo = Some(state);
    let guard = EchoGuard(&signals.echo);
    let disabled = Command::new("stty").arg("-echo").stdin(Stdio::inherit()).stdout(Stdio::null()).stderr(Stdio::null()).status();
    drop(echo);
    if !disabled?.success() {
        return Err(invalid("could not secure terminal input"));
    }
    eprint!("{prompt}");
    let mut line = String::new();
    let result = std::io::stdin().lock().read_line(&mut line);
    drop(guard);
    eprintln!();
    result?;
    let value = line.trim().to_string();
    if value.is_empty() {
        return Err(invalid("required input was empty"));
    }
    Ok(value)
}
fn input(name: &str, prompt: &str, signals: &mut LoginSignals) -> Result<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v.trim().to_string()),
        _ => hidden(prompt, signals),
    }
}

fn callback_url(callback: &str, redirect: &url::Url, state: &str) -> Result<String> {
    let url = url::Url::parse(callback).map_err(|_| invalid("invalid authorization callback"))?;
    if url.scheme() != redirect.scheme()
        || url.host_str() != redirect.host_str()
        || url.port_or_known_default() != redirect.port_or_known_default()
        || url.path() != redirect.path()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid("invalid authorization callback"));
    }
    parse_callback(url.as_str(), state).map_err(|_| invalid("invalid authorization callback"))
}

fn request_callback(line: &[u8], redirect: &url::Url, state: &str) -> Result<String> {
    if line.len() > 8192 || !line.ends_with(b"\n") {
        return Err(invalid("invalid authorization callback"));
    }
    let line = std::str::from_utf8(line).map_err(|_| invalid("invalid authorization callback"))?;
    let parts: Vec<_> = line.split_whitespace().collect();
    if parts.len() != 3 || parts[0] != "GET" || !parts[2].starts_with("HTTP/1.") || !parts[1].starts_with('/') || parts[1].starts_with("//")
    {
        return Err(invalid("invalid authorization callback"));
    }
    let joined = redirect.join(parts[1]).map_err(|_| invalid("invalid authorization callback"))?;
    callback_url(joined.as_str(), redirect, state)
}

fn receive(listener: Option<TcpListener>, redirect: &url::Url, state: &str, signals: &mut LoginSignals) -> Result<String> {
    if let Some(listener) = listener {
        eprintln!("Waiting for browser authorization.");
        for stream in listener.incoming() {
            let mut stream = stream?;
            let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
            let mut line = Vec::new();
            let mut reader = std::io::BufReader::new(stream.try_clone()?);
            let read = reader.by_ref().take(8193).read_until(b'\n', &mut line);
            let code = read.ok().and_then(|_| request_callback(&line, redirect, state).ok());
            let response = if code.is_some() {
                "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nYou may close this window.\r\n"
            } else {
                "HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid request.\r\n"
            };
            let _ = stream.write_all(response.as_bytes());
            if let Some(code) = code {
                return Ok(code);
            }
        }
        Err(invalid("authorization callback was not received"))
    } else {
        let callback = hidden("Paste the final browser address (hidden): ", signals)?;
        callback_url(&callback, redirect, state)
    }
}

struct Memory(Mutex<Option<Tokens>>);
impl TokenStore for Memory {
    fn load(&self) -> Result<Option<Tokens>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, t: &Tokens) -> Result<()> {
        *self.0.lock().unwrap() = Some(t.clone());
        Ok(())
    }
}

const AUTH_MARKER: &str = "snapshot-auth.json";
const AUTH_PURPOSE: &str = "inoink-read-only-snapshot";

fn require_marker(auth: &Path) -> Result<()> {
    let path = auth.join(AUTH_MARKER);
    private_file(&path).map_err(|_| invalid("snapshot authorization marker missing or not private"))?;
    let data = std::fs::read(&path).map_err(|_| invalid("invalid snapshot authorization marker"))?;
    let marker: serde_json::Value = serde_json::from_slice(&data).map_err(|_| invalid("invalid snapshot authorization marker"))?;
    if marker != serde_json::json!({"format": 1, "purpose": AUTH_PURPOSE}) {
        return Err(invalid("invalid snapshot authorization marker"));
    }
    Ok(())
}

pub fn login(args: &[String]) -> Result<()> {
    let mut signals = LoginSignals::new()?;
    let (auth, _) = arguments(args, false)?;
    private_dir(&auth, true)?;
    if exists(&auth.join("credentials.json"))? || exists(&auth.join(AUTH_MARKER))? {
        return Err(invalid("snapshot authorization already exists; remove it only when no operation is active"));
    }
    let client = input("INOINK_CLIENT_ID", "Client ID (hidden): ", &mut signals)?;
    let secret = input("INOINK_CLIENT_SECRET", "Client secret (hidden): ", &mut signals)?;
    let redirect = input("INOINK_REDIRECT_URI", "Registered callback URI (hidden): ", &mut signals)?;
    let redirect_url = url::Url::parse(&redirect).map_err(|_| invalid("invalid registered callback URI"))?;
    if redirect_url.fragment().is_some()
        || redirect_url.query().is_some()
        || !redirect_url.username().is_empty()
        || redirect_url.password().is_some()
    {
        return Err(invalid("invalid registered callback URI"));
    }
    let cfg = AppConfig::new(&client, &secret, &redirect);
    let state = super::random_hex(16)?;
    let listener = if redirect_url.scheme() == "http" && matches!(redirect_url.host_str(), Some("localhost" | "127.0.0.1")) {
        redirect_url.port().and_then(|port| TcpListener::bind(("127.0.0.1", port)).ok())
    } else {
        None
    };
    let consent = authorize_url(&cfg, &state, "read");
    let opened = Command::new(if cfg!(target_os = "macos") { "open" } else { "xdg-open" })
        .arg(consent)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !opened.is_ok_and(|s| s.success()) {
        return Err(invalid("could not open the browser; configure a browser and try again"));
    }
    let code = receive(listener, &redirect_url, &state, &mut signals)?;
    finish_login(&auth, cfg, &code, Arc::new(UreqTransport::new()), Arc::new(SystemClock))?;
    println!("Read-only snapshot authorization saved.");
    Ok(())
}

fn finish_login(
    auth: &Path,
    cfg: AppConfig,
    code: &str,
    inner: Arc<dyn inoink::api::Transport>,
    clock: Arc<dyn inoink::api::Clock>,
) -> Result<()> {
    let transport = Arc::new(ReadOnlyTransport::new(inner, &cfg)?);
    let tokens = exchange_code(&cfg, transport.as_ref(), clock.as_ref(), code).map_err(|_| safe_failure())?;
    snapshot::require_read_scope(&tokens)?;
    let memory = Arc::new(ReadOnlyTokens::new(Arc::new(Memory(Mutex::new(Some(tokens))))));
    let api = ApiClient::new(cfg.clone(), transport, clock, memory.clone()).map_err(|_| safe_failure())?;
    let info = api.user_info().map_err(|_| safe_failure())?;
    let tokens = memory.load()?.ok_or(Error::NeedsAuthorization)?;
    let mut credentials = Credentials::new(&cfg, &tokens);
    credentials.user_id = info.user_id;
    let path = auth.join("credentials.json");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|_| invalid("credentials already exist or cannot be created"))?;
    if FileCredentials::new(auth).save_credentials(&credentials).is_err() {
        let _ = std::fs::remove_file(&path);
        return Err(safe_failure());
    }
    private_file(&path)?;
    let marker_path = auth.join(AUTH_MARKER);
    let mut marker = match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&marker_path) {
        Ok(marker) => marker,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return Err(safe_failure());
        }
    };
    if marker.write_all(serde_json::json!({"format": 1, "purpose": AUTH_PURPOSE}).to_string().as_bytes()).is_err() {
        let _ = std::fs::remove_file(&marker_path);
        let _ = std::fs::remove_file(&path);
        return Err(safe_failure());
    }
    Ok(())
}

// Atomic no-replace publication: never overwrite another process's output.
fn publish(source: &Path, destination: &Path) -> std::io::Result<()> {
    let from = std::ffi::CString::new(source.as_os_str().as_bytes()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let to =
        std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn renamex_np(from: *const std::ffi::c_char, to: *const std::ffi::c_char, flags: u32) -> i32;
    }
    #[cfg(target_os = "linux")]
    unsafe extern "C" {
        fn renameat2(olddir: i32, from: *const std::ffi::c_char, newdir: i32, to: *const std::ffi::c_char, flags: u32) -> i32;
    }
    #[cfg(target_os = "macos")]
    let result = unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 4) }; // RENAME_EXCL
    #[cfg(target_os = "linux")]
    let result = unsafe { renameat2(-100, from.as_ptr(), -100, to.as_ptr(), 1) }; // AT_FDCWD, RENAME_NOREPLACE
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result = {
        let _ = (from, to);
        return Err(std::io::Error::from(std::io::ErrorKind::Unsupported));
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let (auth, out) = arguments(args, true)?;
    let out = out.unwrap();
    separate(&auth, &out)?;
    if exists(&out)? {
        return Err(invalid("snapshot output already exists"));
    }
    private_dir(&auth, false)?;
    require_marker(&auth)?;
    private_file(&auth.join("credentials.json"))?;
    let file = Arc::new(FileCredentials::new(&auth));
    let creds = file.load_credentials().map_err(|_| safe_failure())?.ok_or(Error::NeedsAuthorization)?;
    snapshot::require_read_scope(&creds.tokens())?;
    let cfg = creds.app_config();
    let parent = out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    // A private staging directory makes failure cleanup ownership unambiguous.
    let stage = loop {
        let candidate = parent.join(format!(".inoink-snapshot-{}", super::random_hex(16)?));
        match std::fs::DirBuilder::new().mode(0o700).create(&candidate) {
            Ok(()) => break candidate,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(safe_failure()),
        }
    };
    let data = stage.join("data");
    let report = snapshot::download(
        &data,
        cfg.clone(),
        Arc::new(UreqTransport::new()),
        Arc::new(SystemClock),
        file,
        Arc::new(UreqFetcher::new(&cfg.user_agent)),
    );
    let result = match report {
        Ok(report) => {
            if publish(&data, &out).is_err() {
                Err(safe_failure())
            } else {
                println!("Snapshot downloaded: {} articles, {} with incomplete images.", report.articles, report.incomplete_images);
                Ok(())
            }
        }
        Err(_) => Err(safe_failure()),
    };
    let _ = std::fs::remove_dir_all(&stage);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("inoink-cli-test-{}", super::super::random_hex(16).unwrap()));
            std::fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn strict_arguments() {
        assert!(arguments(&args(&["--auth", "a"]), false).is_ok());
        assert!(arguments(&args(&["--output", "b", "--auth", "a"]), true).is_ok());
        for a in [
            &[][..],
            &["--auth"][..],
            &["--auth", "a", "--auth", "b"][..],
            &["--auth", "a", "--secret", "x"][..],
            &["--auth", "--output", "b"][..],
        ] {
            assert!(arguments(&args(a), false).is_err());
        }
        assert!(arguments(&args(&["--auth", "a"]), true).is_err());
    }

    #[test]
    fn overlap_resolves_symlinks_and_parent_components() {
        let temp = Temp::new();
        let auth = temp.0.join("auth");
        std::fs::create_dir(&auth).unwrap();
        symlink(&auth, temp.0.join("alias")).unwrap();
        assert!(separate(&auth, &temp.0.join("alias/new/data")).is_err());
        assert!(separate(&auth, &temp.0).is_err());
        assert!(separate(&auth, &temp.0.join("missing/../auth/data")).is_err());
        assert!(separate(&auth, &temp.0.join("data")).is_ok());
    }

    #[test]
    fn private_permissions_and_symlinks() {
        let temp = Temp::new();
        let auth = temp.0.join("auth");
        private_dir(&auth, true).unwrap();
        assert_eq!(std::fs::metadata(&auth).unwrap().permissions().mode() & 0o777, 0o700);
        let file = auth.join("credentials.json");
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file).unwrap();
        private_file(&file).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(private_file(&file).is_err());
        symlink(&auth, temp.0.join("link")).unwrap();
        assert!(private_dir(&temp.0.join("link"), false).is_err());
    }

    #[test]
    fn output_refusal_precedes_auth_and_preserves_data() {
        let temp = Temp::new();
        let output = temp.0.join("output");
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("keep"), b"keep").unwrap();
        let result = run(&args(&["--auth", temp.0.join("missing").to_str().unwrap(), "--output", output.to_str().unwrap()]));
        assert!(result.is_err());
        assert_eq!(std::fs::read(output.join("keep")).unwrap(), b"keep");
    }

    #[test]
    fn publication_never_replaces_existing_directory() {
        let temp = Temp::new();
        let source = temp.0.join("source");
        let dest = temp.0.join("dest");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&dest).unwrap();
        assert!(publish(&source, &dest).is_err());
        assert!(source.is_dir() && dest.is_dir());
    }

    struct FakeTransport {
        scope: &'static str,
        calls: Mutex<usize>,
    }
    impl inoink::api::Transport for FakeTransport {
        fn send(&self, req: &inoink::api::HttpRequest) -> Result<inoink::api::HttpResponse> {
            *self.calls.lock().unwrap() += 1;
            let body = if req.url.ends_with("/oauth2/token") {
                serde_json::json!({"access_token":"fake-access", "refresh_token":"fake-refresh", "expires_in":3600, "scope":self.scope})
            } else {
                serde_json::json!({"userId":"fake-user", "userName":"test", "userEmail":""})
            };
            Ok(inoink::api::HttpResponse { status: 200, headers: vec![], body: body.to_string().into_bytes() })
        }
    }

    #[test]
    fn fake_login_saves_private_credentials_and_rejects_scope_before_user_info() {
        for (scope, valid) in [("read", true), ("read write", false), ("", false)] {
            let temp = Temp::new();
            let cfg = AppConfig::new("id", "secret", "http://localhost:8888/callback");
            let transport = Arc::new(FakeTransport { scope, calls: Mutex::new(0) });
            let result = finish_login(&temp.0, cfg, "fake-code", transport.clone(), Arc::new(SystemClock));
            assert_eq!(result.is_ok(), valid);
            assert_eq!(*transport.calls.lock().unwrap(), if valid { 2 } else { 1 });
            let path = temp.0.join("credentials.json");
            assert_eq!(path.exists(), valid);
            if valid {
                private_file(&path).unwrap();
                require_marker(&temp.0).unwrap();
                let c = FileCredentials::new(&temp.0).load_credentials().unwrap().unwrap();
                assert_eq!(c.scope, "read");
                assert_eq!(c.user_id, "fake-user");
                // A second login cannot replace the existing credentials.
                let cfg = AppConfig::new("other", "secret", "http://localhost:8888/callback");
                assert!(finish_login(&temp.0, cfg, "fake-code", transport.clone(), Arc::new(SystemClock)).is_err());
                assert_eq!(FileCredentials::new(&temp.0).load_credentials().unwrap().unwrap().client_id, "id");
            }
        }
    }

    #[test]
    fn invalid_scope_is_rejected_before_output_creation() {
        let temp = Temp::new();
        let auth = temp.0.join("auth");
        private_dir(&auth, true).unwrap();
        let cfg = AppConfig::new("id", "secret", "http://localhost:8888/callback");
        let tokens = Tokens { access_token: "a".into(), refresh_token: "r".into(), expires_at: 0, scope: "read write".into() };
        FileCredentials::new(&auth).save_credentials(&Credentials::new(&cfg, &tokens)).unwrap();
        std::fs::write(auth.join(AUTH_MARKER), serde_json::json!({"format": 1, "purpose": AUTH_PURPOSE}).to_string()).unwrap();
        std::fs::set_permissions(auth.join(AUTH_MARKER), std::fs::Permissions::from_mode(0o600)).unwrap();
        let out = temp.0.join("output");
        let result = run(&args(&["--auth", auth.to_str().unwrap(), "--output", out.to_str().unwrap()]));
        assert!(result.is_err());
        assert!(!out.exists());
    }

    #[test]
    fn marker_required_and_private_before_credentials_are_touched() {
        let temp = Temp::new();
        let auth = temp.0.join("auth");
        private_dir(&auth, true).unwrap();
        let credentials = auth.join("credentials.json");
        std::fs::write(&credentials, b"production credentials unchanged").unwrap();
        std::fs::set_permissions(&credentials, std::fs::Permissions::from_mode(0o600)).unwrap();
        let out = temp.0.join("output");
        let argv = args(&["--auth", auth.to_str().unwrap(), "--output", out.to_str().unwrap()]);
        assert!(run(&argv).is_err());
        assert_eq!(std::fs::read(&credentials).unwrap(), b"production credentials unchanged");
        assert!(!out.exists());
        let marker = auth.join(AUTH_MARKER);
        for body in [r#"{}"#, r#"{"format":2,"purpose":"inoink-read-only-snapshot"}"#, r#"{"format":1,"purpose":"other"}"#] {
            std::fs::write(&marker, body).unwrap();
            std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(require_marker(&auth).is_err());
        }
        std::fs::write(&marker, serde_json::json!({"format": 1, "purpose": AUTH_PURPOSE}).to_string()).unwrap();
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(require_marker(&auth).is_err());
        std::fs::set_permissions(&marker, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(require_marker(&auth).is_ok());
        std::fs::remove_file(&marker).unwrap();
        symlink(&credentials, &marker).unwrap();
        assert!(require_marker(&auth).is_err());
        assert!(login(&args(&["--auth", auth.to_str().unwrap()])).is_err());
        std::fs::remove_file(&credentials).unwrap();
        assert!(login(&args(&["--auth", auth.to_str().unwrap()])).is_err());
        assert!(std::fs::symlink_metadata(&marker).is_ok());
    }

    #[test]
    fn callback_checks_path_state_method_and_size() {
        let redirect = url::Url::parse("http://127.0.0.1:8888/callback").unwrap();
        assert_eq!(request_callback(b"GET /callback?code=fake&state=abc HTTP/1.1\r\n", &redirect, "abc").unwrap(), "fake");
        for line in [
            "POST /callback?code=fake&state=abc HTTP/1.1\r\n",
            "GET /other?code=fake&state=abc HTTP/1.1\r\n",
            "GET /callback?code=fake&state=wrong HTTP/1.1\r\n",
            "GET //evil/callback?code=fake&state=abc HTTP/1.1\r\n",
            "GET /callback?code=fake&state=abc HTTP/1.1",
        ] {
            assert!(request_callback(line.as_bytes(), &redirect, "abc").is_err());
        }
        assert!(request_callback(&vec![b'x'; 8193], &redirect, "abc").is_err());
        assert!(callback_url("http://evil.invalid/callback?code=private&state=abc", &redirect, "abc").is_err());
        assert!(!callback_url("http://evil.invalid/callback?code=private&state=abc", &redirect, "abc")
            .unwrap_err()
            .to_string()
            .contains("private"));
    }
}
