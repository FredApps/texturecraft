//! Tokens and bounded framing for the loopback JSON-lines transports (the app's control channel
//! and `texturecraft-cli serve` share the token conventions).

use std::io::{BufRead, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::time::Duration;

use crate::{AutomationError, bad};

/// Environment variable holding the control token (both sides).
pub const TOKEN_ENV: &str = "TEXTURECRAFT_CONTROL_TOKEN";
/// Largest JSON request line, newline included (the app's limit too).
pub const MAX_REQUEST_BYTES: usize = 8 << 20;
/// Simultaneous TCP connections served by `serve --port`.
pub const MAX_CONNECTIONS: usize = 16;
/// Largest token file read (a token is at most 1024 characters, plus whitespace).
const MAX_TOKEN_FILE_BYTES: u64 = 4096;
/// Time a TCP client has to authenticate before its connection is closed.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Steps in one `command_batch`.
pub const MAX_BATCH_STEPS: usize = 256;

/// The token file the app (and `serve --port`) writes when the environment gives no token.
pub fn token_file(port: u16) -> PathBuf {
    std::env::temp_dir().join(format!("texturecraft-control-{port}.token"))
}

/// The app accepts any environment token of at least 16 characters; generated ones are 64 hex.
fn validate_token(t: &str) -> Result<(), AutomationError> {
    if (16..=1024).contains(&t.len()) && t.bytes().all(|b| b.is_ascii_graphic()) {
        Ok(())
    } else {
        Err(bad("control token must be 16 to 1024 printable ASCII characters"))
    }
}

/// A fresh 256-bit token as 64 hex digits, from the OS random generator.
pub fn generate_token() -> Result<String, AutomationError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| AutomationError::Other(format!("cannot generate a control token: {e}")))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Token for a client of `port`: explicit, else `TEXTURECRAFT_CONTROL_TOKEN`, else the token file.
pub fn client_token(port: u16, supplied: Option<String>) -> Result<String, AutomationError> {
    let token = match supplied.or_else(|| std::env::var(TOKEN_ENV).ok()) {
        Some(t) => t,
        None => {
            let path = token_file(port);
            read_token_file(&path)
                .map_err(|e| bad(format!("no control token: set {TOKEN_ENV} or start the app with `--control {port}` so it writes {} ({e})", path.display())))?
        }
    };
    let token = token.trim().to_string();
    validate_token(&token)?;
    Ok(token)
}

/// Token for a server on `port`: `TEXTURECRAFT_CONTROL_TOKEN`, else a fresh one written to
/// [`token_file`] (owner-only on Unix). Returns the token and where it was written. Call it
/// once the port is bound, so a server that cannot listen never replaces another's token.
pub fn server_token(port: u16) -> Result<(String, Option<PathBuf>), AutomationError> {
    if let Ok(t) = std::env::var(TOKEN_ENV) {
        validate_token(t.trim())?;
        return Ok((t.trim().to_string(), None));
    }
    let token = generate_token()?;
    let path = token_file(port);
    write_token_file(&path, &token).map_err(|e| {
        AutomationError::Io(format!("cannot write the control token to {} ({e}); set {TOKEN_ENV} to a token of your own instead", path.display()))
    })?;
    Ok((token, Some(path)))
}

/// Write `token` to a new file at `path`, readable by the owner only on Unix. Whatever was at
/// `path` is removed first and the file is created exclusively (`O_EXCL`, which never follows
/// a symlink), so a file or link planted in a shared temp folder can neither keep its
/// permissions nor redirect the write; one that cannot be removed (another user's, in a sticky
/// `/tmp`) fails the call.
fn write_token_file(path: &Path, token: &str) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let mut f = o.open(path)?;
    f.write_all(token.as_bytes())?;
    f.sync_all()
}

/// Read a token file written by [`write_token_file`]. On Unix the file must be a regular file
/// (not a symlink, FIFO or device) owned by this user with no group or other permissions, so a
/// file another user planted is never trusted and a link can't make a client send one of its
/// own secrets as the token.
fn read_token_file(path: &Path) -> std::io::Result<String> {
    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags};
        use std::os::unix::fs::MetadataExt;
        let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        let file = std::fs::File::from(rustix::fs::open(path, flags, Mode::empty())?);
        let meta = file.metadata()?;
        let refuse = |why: &str| Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, format!("refusing the token file: {why}")));
        if !meta.is_file() {
            return refuse("not a regular file");
        }
        if meta.uid() != rustix::process::geteuid().as_raw() {
            return refuse("owned by another user");
        }
        if meta.mode() & 0o077 != 0 {
            return refuse("readable or writable by other users");
        }
        file
    };
    #[cfg(not(unix))]
    let file = std::fs::File::open(path)?;
    let mut out = String::new();
    file.take(MAX_TOKEN_FILE_BYTES).read_to_string(&mut out)?;
    Ok(out)
}

/// Compare equal-length tokens without an early exit.
pub fn token_matches(expected: &str, supplied: &str) -> bool {
    expected.len() == supplied.len() && expected.bytes().zip(supplied.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

pub enum LineRead {
    Eof,
    Line,
    /// Over [`MAX_REQUEST_BYTES`]; the rest of the line is still unread.
    TooLong,
}

/// Read one line, never buffering more than [`MAX_REQUEST_BYTES`] + 1 bytes.
pub fn read_bounded_line(r: &mut impl BufRead, line: &mut String) -> std::io::Result<LineRead> {
    line.clear();
    let mut bytes = Vec::new();
    let n = std::io::Read::take(&mut *r, (MAX_REQUEST_BYTES + 1) as u64).read_until(b'\n', &mut bytes)?;
    if n == 0 {
        return Ok(LineRead::Eof);
    }
    if n > MAX_REQUEST_BYTES {
        if bytes.last() != Some(&b'\n') {
            discard_rest_of_line(r)?;
        }
        return Ok(LineRead::TooLong);
    }
    *line = String::from_utf8(bytes).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(LineRead::Line)
}

fn discard_rest_of_line(r: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            return Ok(());
        }
        if let Some(end) = buf.iter().position(|&b| b == b'\n') {
            r.consume(end + 1);
            return Ok(());
        }
        let n = buf.len();
        r.consume(n);
    }
}

/// Counts live connections; a permit is released when dropped.
#[derive(Clone, Default)]
pub struct ConnectionLimiter(Arc<AtomicUsize>);

pub struct Permit(Arc<AtomicUsize>);

impl ConnectionLimiter {
    pub fn try_acquire(&self, max_connections: usize) -> Option<Permit> {
        self.0
            .try_update(Ordering::AcqRel, Ordering::Acquire, |connections| (connections < max_connections).then_some(connections + 1))
            .ok()
            .map(|_| Permit(self.0.clone()))
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Closes a connection that has not authenticated in time, so idle or trickling clients can't
/// hold the few connection slots. Drop it once the client is authenticated (or gone).
pub struct AuthDeadline {
    _cancel: Sender<()>,
}

impl AuthDeadline {
    /// Shut `stream` down after `timeout` unless the returned deadline is dropped first.
    pub fn start(stream: &TcpStream, timeout: Duration) -> std::io::Result<AuthDeadline> {
        let stream = stream.try_clone()?;
        let (tx, rx) = channel::<()>();
        std::thread::spawn(move || {
            // Dropping the deadline disconnects the channel and ends this thread at once.
            if let Err(RecvTimeoutError::Timeout) = rx.recv_timeout(timeout) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        Ok(AuthDeadline { _cancel: tx })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        let a = generate_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, generate_token().unwrap());
        assert!(token_matches(&a, &a.clone()));
        assert!(!token_matches(&a, "short"));
        assert!(client_token(1, Some("too short".into())).is_err());
        assert!(client_token(1, Some(format!(" {a}\n"))).is_ok());
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-token-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn token_files_round_trip_and_replace_old_ones() {
        let dir = scratch("round-trip");
        let path = dir.join("t.token");
        write_token_file(&path, "first-token-0123456789").unwrap();
        assert_eq!(read_token_file(&path).unwrap(), "first-token-0123456789");
        write_token_file(&path, "second-token-0123456789").unwrap();
        assert_eq!(read_token_file(&path).unwrap(), "second-token-0123456789");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn token_files_are_owner_only_even_over_an_open_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perms");
        let path = dir.join("t.token");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        write_token_file(&path, "fresh-token-0123456789").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn token_files_never_write_through_a_symlink() {
        let dir = scratch("symlink-write");
        let (path, victim) = (dir.join("t.token"), dir.join("victim"));
        std::fs::write(&victim, "keep me").unwrap();
        std::os::unix::fs::symlink(&victim, &path).unwrap();
        write_token_file(&path, "fresh-token-0123456789").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep me");
        assert!(!std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn unsafe_token_files_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("refuse");
        // Readable by others: another user may have planted or read it.
        let open = dir.join("open.token");
        std::fs::write(&open, "planted-token-0123456789").unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_token_file(&open).is_err());
        // A symlink, even to an owner-only file: a client must not send that file as its token.
        let secret = dir.join("secret");
        write_token_file(&secret, "private-0123456789abcdef").unwrap();
        let link = dir.join("link.token");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(read_token_file(&link).is_err());
        // A FIFO is refused without blocking on open or read.
        let fifo = dir.join("fifo.token");
        if std::process::Command::new("mkfifo").arg(&fifo).status().is_ok_and(|s| s.success()) {
            assert!(read_token_file(&fifo).is_err());
        }
        // A directory is not a token.
        assert!(read_token_file(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_lines_are_skipped() {
        let input = format!("{}\n{{\"ok\":1}}\n", "x".repeat(MAX_REQUEST_BYTES + 10));
        let mut r = std::io::Cursor::new(input);
        let mut line = String::new();
        assert!(matches!(read_bounded_line(&mut r, &mut line).unwrap(), LineRead::TooLong));
        assert!(matches!(read_bounded_line(&mut r, &mut line).unwrap(), LineRead::Line));
        assert_eq!(line, "{\"ok\":1}\n");
        assert!(matches!(read_bounded_line(&mut r, &mut line).unwrap(), LineRead::Eof));
    }

    #[test]
    fn auth_deadline_closes_idle_connections_only() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let idle = TcpStream::connect(addr).unwrap();
        let (server_idle, _) = listener.accept().unwrap();
        let _deadline = AuthDeadline::start(&server_idle, Duration::from_millis(200)).unwrap();
        let mut buf = [0u8; 1];
        idle.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(std::io::Read::read(&mut &idle, &mut buf).unwrap(), 0, "closed by the deadline");

        let kept = TcpStream::connect(addr).unwrap();
        let (server_kept, _) = listener.accept().unwrap();
        drop(AuthDeadline::start(&server_kept, Duration::from_millis(200)).unwrap());
        std::thread::sleep(Duration::from_millis(400));
        (&server_kept).write_all(b"x").unwrap();
        kept.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(std::io::Read::read(&mut &kept, &mut buf).unwrap(), 1, "a met deadline leaves the connection open");
    }

    #[test]
    fn limiter_releases() {
        let limiter = ConnectionLimiter::default();

        let permit = limiter.try_acquire(1).unwrap();
        assert!(limiter.try_acquire(1).is_none());

        drop(permit);
        assert!(limiter.try_acquire(1).is_some());
    }
}
