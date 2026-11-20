//! What a run may use: its own scope with a cap on processes and threads, and a memory limit.

use super::*;

/// Processes and threads a run may have at once, when it runs in a scope of its own.
pub(super) const MAX_TASKS: u32 = 256;

/// What `systemd-run` needs to reach the user's session; bubblewrap removes them again.
pub(super) const SCOPE_VARS: [&str; 2] = ["XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"];

/// The arguments that put a run in a transient systemd scope with a task cap.
pub(super) fn scope_args() -> Vec<String> {
    vec![
        "--user".into(),
        "--scope".into(),
        "--quiet".into(),
        "--collect".into(),
        "-p".into(),
        format!("TasksMax={MAX_TASKS}"),
        "--".into(),
    ]
}

/// `systemd-run`, when this session can start a user scope with it (checked once). Not on macOS
/// or Windows, nor in containers without a user session; there a run has its time limit and its
/// own PID namespace instead.
pub(super) fn task_scope() -> Option<&'static Path> {
    static SCOPE: OnceLock<Option<PathBuf>> = OnceLock::new();
    SCOPE
        .get_or_init(|| {
            if !cfg!(target_os = "linux") {
                return None;
            }
            let path = ["/usr/bin/systemd-run", "/bin/systemd-run"]
                .iter()
                .map(PathBuf::from)
                .find(|p| p.exists())?;
            let works = std::process::Command::new(&path)
                .args(scope_args())
                .arg("true")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            works.then_some(path)
        })
        .as_deref()
}

/// The least memory a run is allowed, however small the computer.
#[cfg(target_os = "linux")]
pub(super) const MIN_MEMORY: u64 = 1 << 30;

/// The most memory a run may hold: half of this computer's, so a runaway program fails on its
/// own instead of pushing the desktop into swap.
#[cfg(target_os = "linux")]
pub(super) fn memory_limit() -> u64 {
    // SAFETY: `sysconf` has no preconditions.
    let (pages, page) = unsafe {
        (
            libc::sysconf(libc::_SC_PHYS_PAGES),
            libc::sysconf(libc::_SC_PAGESIZE),
        )
    };
    let total = if pages > 0 && page > 0 {
        pages as u64 * page as u64
    } else {
        0
    };
    (total / 2).max(MIN_MEMORY)
}

/// Caps a run's heap and other private writable memory (`RLIMIT_DATA`) at `bytes`, and turns
/// off core dumps. Not the address space (`RLIMIT_AS`): runtimes such as V8 reserve far more of
/// it than they use. There is no process limit: `RLIMIT_NPROC` counts every process the user
/// runs, not only the run's, so it would stop runs on a busy desktop. The run's PID namespace
/// and time limit bound what it starts instead.
#[cfg(target_os = "linux")]
pub(super) fn limit_memory(command: &mut tokio::process::Command, bytes: u64) {
    // SAFETY: the closure runs in the child between fork and exec, and calls only `getrlimit`
    // and `setrlimit`, which are async-signal-safe and touch no memory the parent shares.
    unsafe {
        command.pre_exec(move || {
            let mut current = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(libc::RLIMIT_DATA, &mut current) == 0 {
                // A process may lower its hard limit, never raise it.
                let cap = (bytes as libc::rlim_t).min(current.rlim_max);
                let data = libc::rlimit {
                    rlim_cur: cap,
                    rlim_max: cap,
                };
                libc::setrlimit(libc::RLIMIT_DATA, &data);
            }
            let core = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            libc::setrlimit(libc::RLIMIT_CORE, &core);
            Ok(())
        });
    }
}
