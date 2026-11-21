mod databases;

use databases::Databases;
use std::{
    cell::{Cell, RefCell},
    fs::File,
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::Path,
    rc::Rc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    task::{self, LocalSet},
    time::{sleep, timeout},
};

const SOCKET: &str = "/run/rust-geoip/lookup.sock";
const DATA: &str = "/var/lib/rust-geoip";
const IDLE: Duration = Duration::from_secs(10);

struct State {
    databases: RefCell<Databases>,
    reloading: Cell<bool>,
}

async fn handle(stream: UnixStream, state: Rc<State>) -> io::Result<()> {
    let uid = stream.peer_cred()?.uid();
    let may_reload = uid == 0 || uid == unsafe { libc::geteuid() };
    let mut stream = BufReader::with_capacity(512, stream);
    let mut line = Vec::with_capacity(64);
    let mut out = Vec::with_capacity(512);
    loop {
        line.clear();
        out.clear();
        let n = timeout(IDLE, (&mut stream).take(65).read_until(b'\n', &mut line)).await??;
        if n == 0 || n > 64 || line.last() != Some(&b'\n') {
            return Ok(());
        }
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        match line.as_slice() {
            b"PING" => out
                .extend_from_slice(format!("PONG {}", state.databases.borrow().oldest).as_bytes()),
            b"RELOAD" if !may_reload => out.extend_from_slice(b"DENIED"),
            b"RELOAD" if !state.reloading.replace(true) => {
                // A panicking loader must not leave every later RELOAD answering BUSY.
                let set = task::spawn_blocking(|| Databases::load(Path::new(DATA)))
                    .await
                    .unwrap_or_else(|e| Err(e.into()));
                state.reloading.set(false);
                match set {
                    Ok(set) => {
                        state.databases.replace(set);
                        out.extend_from_slice(b"OK");
                    }
                    Err(e) => {
                        eprintln!("reload failed: {e}");
                        out.extend_from_slice(b"ERROR");
                    }
                }
            }
            b"RELOAD" => out.extend_from_slice(b"BUSY"),
            raw => match std::str::from_utf8(raw).ok().and_then(|s| s.parse().ok()) {
                Some(ip) => state.databases.borrow().answer(ip, &mut out),
                None => out.extend_from_slice(br#"{"error":"invalid_ip"}"#),
            },
        }
        out.push(b'\n');
        timeout(IDLE, stream.get_mut().write_all(&out)).await??;
    }
}

async fn command(reload: bool) -> io::Result<()> {
    let mut s = BufReader::new(UnixStream::connect(SOCKET).await?);
    s.get_mut()
        .write_all(if reload { b"RELOAD\n" } else { b"PING\n" })
        .await?;
    let mut response = String::new();
    s.take(65).read_line(&mut response).await?;
    if reload && response == "OK\n" {
        return Ok(());
    }
    if !reload {
        if let Some(date) = response
            .strip_prefix("PONG ")
            .and_then(|s| s.strip_suffix('\n'))
        {
            if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() {
                println!("{date}");
                return Ok(());
            }
        }
    }
    let message = match response.as_str() {
        "DENIED\n" => "reload requires root or the service account",
        "BUSY\n" => "another reload is in progress",
        "ERROR\n" => "reload failed; see the service journal",
        _ => "unexpected response from rust-geoip",
    };
    Err(io::Error::other(message))
}

/// libGeoIP parses files from a third-party mirror. Before that the validator keeps
/// read access below the checked directory only and loses TCP (Landlock ABI 4+).
fn confine(directory: &Path) -> io::Result<()> {
    #[repr(C)]
    struct Ruleset {
        fs: u64,
        net: u64,
    }
    #[repr(C, packed)]
    struct Beneath {
        access: u64,
        parent: i32,
    }
    let abi = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            std::ptr::null::<Ruleset>(),
            0usize,
            1u32,
        )
    };
    if abi < 1 {
        eprintln!(
            "landlock unavailable, validating unconfined: {}",
            io::Error::last_os_error()
        );
        return Ok(());
    }
    // Every ABI 1 filesystem right is denied; READ_FILE | READ_DIR stay below the directory.
    let ruleset = Ruleset {
        fs: (1 << 13) - 1,
        net: if abi >= 4 { 0b11 } else { 0 },
    };
    let fd = unsafe {
        libc::syscall(
            libc::SYS_landlock_create_ruleset,
            &ruleset,
            std::mem::size_of::<Ruleset>(),
            0u32,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
    let root = File::open(directory)?;
    let rule = Beneath {
        access: (1 << 2) | (1 << 3),
        parent: root.as_raw_fd(),
    };
    unsafe {
        if libc::syscall(
            libc::SYS_landlock_add_rule,
            fd.as_raw_fd(),
            1u32,
            &rule,
            0u32,
        ) != 0
            || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            || libc::syscall(libc::SYS_landlock_restrict_self, fd.as_raw_fd(), 0u32) != 0
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() == 2 && args[0] == "validate" {
        let directory = Path::new(&args[1]);
        confine(directory)?;
        println!("{}", Databases::load(directory)?.oldest);
        return Ok(());
    }
    if args.len() == 1 && matches!(args[0].as_str(), "ping" | "reload") {
        let reload = args[0] == "reload";
        return runtime.block_on(async {
            timeout(
                Duration::from_secs(if reload { 30 } else { 1 }),
                command(reload),
            )
            .await?
        });
    }
    if !args.is_empty() {
        return Err(io::Error::other(
            "usage: rust-geoip [ping|reload|validate DIRECTORY]",
        ));
    }
    LocalSet::new().block_on(&runtime, async {
        // An update published during startup can queue a reload on this socket.
        let listener = UnixListener::bind(SOCKET)?;
        let state = Rc::new(State {
            databases: RefCell::new(Databases::load(Path::new(DATA))?),
            reloading: Cell::new(false),
        });
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    task::spawn_local(handle(stream, Rc::clone(&state)));
                }
                // EMFILE and similar refusals are transient: pause so accepted
                // connections can finish and release descriptors, keep serving.
                Err(e) => {
                    eprintln!("accept failed: {e}");
                    sleep(Duration::from_millis(100)).await;
                }
            }
        }
    })
}
