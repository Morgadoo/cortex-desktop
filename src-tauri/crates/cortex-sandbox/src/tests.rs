use super::*;

async fn js(
    script: &str,
    args: &str,
    caps: Capabilities,
    limits: Limits,
) -> Result<String, SandboxError> {
    run(script, args, &caps, limits, None).await.0
}

#[tokio::test]
async fn a_script_computes_and_returns_json() {
    let out = js(
        "function main(a) { return { sum: a.xs.reduce((s, x) => s + x, 0), n: a.xs.length }; }",
        r#"{"xs":[1,2,3,4]}"#,
        Capabilities::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(out, r#"{"sum":10,"n":4}"#);
}

#[tokio::test]
async fn arguments_never_become_code() {
    let out = js(
        "function main(a) { return a.text; }",
        r#"{"text":"\")); throw new Error(\"x"}"#,
        Capabilities::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert!(out.contains("throw new Error"), "{out}");
}

#[tokio::test]
async fn a_script_error_comes_back_as_text() {
    let err = js(
        "function main() { throw new Error('bad input'); }",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, SandboxError::Script(m) if m.contains("bad input")),
        "{err:?}"
    );
}

#[tokio::test]
async fn an_endless_loop_is_stopped() {
    let err = js(
        "function main() { while (true) {} }",
        "{}",
        Capabilities::default(),
        Limits {
            fuel: 200_000_000,
            ..Limits::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, SandboxError::OutOfFuel | SandboxError::Timeout),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_memory_hog_is_stopped() {
    let err = js(
        "function main() { const a = []; while (true) { a.push('x'.repeat(1 << 20) + a.length); } }",
        "{}",
        Capabilities::default(),
        Limits {
            fuel: u64::MAX,
            memory_bytes: 16 << 20,
            ..Limits::default()
        },
    )
    .await
    .unwrap_err();
    // The limiter fired: not a script error or a crash that only happens to follow.
    assert_eq!(err, SandboxError::OutOfMemory);
}

/// The host-call timeout: a tool waiting on a host function that never answers is stopped.
#[tokio::test]
async fn a_host_call_that_never_returns_is_stopped() {
    let hangs: ToolCaller = std::sync::Arc::new(|_name: String, _args: String| {
        Box::pin(async move {
            std::future::pending::<()>().await;
            Ok(String::new())
        })
    });
    // Compile the runtime first, so the clock below times only the call.
    let _ = js(
        "function main() { return 1; }",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await;
    let started = std::time::Instant::now();
    let (out, _) = run_with_tools(
        "function main() { return tool('slow', {}); }",
        "{}",
        &Capabilities {
            tools: vec!["slow".into()],
            ..Default::default()
        },
        Limits {
            timeout: Duration::from_millis(500),
            ..Limits::default()
        },
        None,
        Some(hangs),
    )
    .await;
    assert_eq!(out.unwrap_err(), SandboxError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// Each call gets a fresh instance: nothing a call leaves behind is there for the next.
#[tokio::test]
async fn nothing_carries_over_between_calls() {
    let script = "function main() { globalThis.count = (globalThis.count || 0) + 1; return globalThis.count; }";
    for _ in 0..2 {
        let out = js(script, "{}", Capabilities::default(), Limits::default()).await;
        assert_eq!(out.unwrap(), "1");
    }
}

/// WASI gives the guest nothing: no environment, arguments, clock-free files or processes.
#[tokio::test]
async fn wasi_gives_nothing() {
    let out = js(
        "function main() { return [typeof process, typeof require, typeof Deno, typeof std, \
         typeof os, typeof fs, typeof XMLHttpRequest, typeof WebSocket].join(','); }",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        out,
        "\"undefined,undefined,undefined,undefined,undefined,undefined,undefined,undefined\""
    );
}

#[tokio::test]
async fn nothing_is_reachable_without_a_capability() {
    let fetch = js(
        "function main() { return fetch('GET', 'https://example.com/', ''); }",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&fetch, SandboxError::Script(m) if m.contains("no http capability")),
        "{fetch:?}"
    );
    let read = js(
        "function main() { return read('notes.txt'); }",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&read, SandboxError::Script(m) if m.contains("denied")),
        "{read:?}"
    );
}

#[tokio::test]
async fn listed_hosts_still_cannot_reach_private_addresses_or_plain_http() {
    let caps = Capabilities {
        http_hosts: vec!["localhost".into(), "127.0.0.1".into()],
        ..Default::default()
    };
    for url in [
        "https://localhost/",
        "https://127.0.0.1/",
        "http://localhost/",
    ] {
        let err = js(
            &format!("function main() {{ return fetch('GET', '{url}', ''); }}"),
            "{}",
            caps.clone(),
            Limits::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, SandboxError::Script(m) if m.contains("denied")),
            "{url}: {err:?}"
        );
    }
}

#[tokio::test]
async fn reads_stay_inside_the_granted_folders_of_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/a.txt"), "hello").unwrap();
    std::fs::write(dir.path().join("secret.txt"), "no").unwrap();
    let caps = Capabilities {
        read_prefixes: vec!["data".into()],
        ..Default::default()
    };
    let read = |path: &str| {
        let caps = caps.clone();
        let script = format!("function main() {{ return read('{path}'); }}");
        let workspace = Workspace {
            root: dir.path().to_path_buf(),
            read: |_, bytes| Ok(bytes),
            write: |_, b| Ok(b.to_vec()),
        };
        async move {
            run(&script, "{}", &caps, Limits::default(), Some(workspace))
                .await
                .0
        }
    };
    assert_eq!(read("data/a.txt").await.unwrap(), "\"hello\"");
    assert!(read("secret.txt").await.is_err());
    assert!(read("../secret.txt").await.is_err());
    assert!(read("data/../secret.txt").await.is_err());
}

/// Write capabilities: only into the granted folders, never out of the workspace or through a
/// link, and only up to the size cap.
#[tokio::test]
async fn writes_stay_inside_the_granted_folders_of_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("out")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), dir.path().join("out/escape")).unwrap();
    let caps = Capabilities {
        write_prefixes: vec!["out".into()],
        ..Default::default()
    };
    let write = |path: &str, text: &str| {
        let (caps, script) = (
            caps.clone(),
            format!("function main() {{ write('{path}', '{text}'); return 'ok'; }}"),
        );
        let workspace = Workspace {
            root: dir.path().to_path_buf(),
            read: |_, bytes| Ok(bytes),
            write: |_, b| Ok(b.to_vec()),
        };
        async move {
            run(&script, "{}", &caps, Limits::default(), Some(workspace))
                .await
                .0
        }
    };
    assert_eq!(write("out/report/a.txt", "hi").await.unwrap(), "\"ok\"");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out/report/a.txt")).unwrap(),
        "hi"
    );
    for (path, why) in [
        ("notes.txt", "no write capability"),
        ("../x.txt", "not a plain relative path"),
        ("/etc/x", "not a plain relative path"),
    ] {
        let err = write(path, "x").await.unwrap_err().to_string();
        assert!(err.contains(why), "{path}: {err}");
    }
    #[cfg(unix)]
    {
        let err = write("out/escape/x.txt", "x")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("leaves the workspace"), "{err}");
        assert!(!outside.path().join("x.txt").exists());
        // A link in the folders on the way: refused before any folder is made past it.
        let err = write("out/escape/new/deeper/x.txt", "x")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("leaves the workspace"), "{err}");
        assert!(!outside.path().join("new").exists(), "nothing made outside");
    }
    // Without the capability nothing is written, even with a workspace.
    let none = run(
        "function main() { write('out/b.txt', 'x'); }",
        "{}",
        &Capabilities::default(),
        Limits::default(),
        Some(Workspace {
            root: dir.path().to_path_buf(),
            read: |_, bytes| Ok(bytes),
            write: |_, b| Ok(b.to_vec()),
        }),
    )
    .await
    .0;
    assert!(none.is_err() && !dir.path().join("out/b.txt").exists());
}

/// The compilation cost: the interpreter is compiled once and kept; later loads read it back,
/// and a damaged copy is compiled again.
#[test]
fn the_compiled_interpreter_is_kept_and_reused() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine().unwrap();
    let kept = || {
        std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "cwasm"))
            .collect::<Vec<_>>()
    };
    load_or_compile(&engine, RUNTIME, Some(dir.path())).unwrap();
    let files = kept();
    assert_eq!(files.len(), 1);
    let started = std::time::Instant::now();
    load_or_compile(&engine, RUNTIME, Some(dir.path())).unwrap();
    let loading = started.elapsed();
    assert!(loading < Duration::from_secs(2), "{loading:?}");
    std::fs::write(&files[0], b"not a component").unwrap();
    load_or_compile(&engine, RUNTIME, Some(dir.path())).unwrap();
    assert!(std::fs::read(&files[0]).unwrap().len() > 1000, "replaced");
}

#[tokio::test]
async fn logs_come_back_apart_from_the_result() {
    let (out, logs) = run(
        "function main() { log('step one'); return 1; }",
        "{}",
        &Capabilities::default(),
        Limits::default(),
        None,
    )
    .await;
    assert_eq!(out.unwrap(), "1");
    assert_eq!(logs, vec!["step one".to_string()]);
}

#[tokio::test]
async fn a_program_without_main_returns_what_it_printed() {
    let out = js(
        "const args = { n: 3 };\nconsole.log('n is', args.n);\nprint({ ok: true });",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(out, "n is 3\n{\"ok\":true}");
}

/// Composition: a script calls another approved tool, only one it has the capability for.
#[tokio::test]
async fn a_script_calls_only_the_tools_it_may() {
    let caller: ToolCaller = std::sync::Arc::new(|name: String, args: String| {
        Box::pin(async move { Ok(format!("{{\"called\":\"{name}\",\"with\":{args}}}")) })
    });
    let script =
        "function main() { const r = tool('double', { x: 2 }); return r.called + ':' + r.with.x; }";
    let allowed = Capabilities {
        tools: vec!["double".into()],
        ..Default::default()
    };
    let (out, _) = run_with_tools(
        script,
        "{}",
        &allowed,
        Limits::default(),
        None,
        Some(caller.clone()),
    )
    .await;
    assert_eq!(out.unwrap(), "\"double:2\"");
    let (denied, _) = run_with_tools(
        script,
        "{}",
        &Capabilities::default(),
        Limits::default(),
        None,
        Some(caller),
    )
    .await;
    assert!(
        matches!(&denied, Err(SandboxError::Script(m)) if m.contains("no capability to call")),
        "{denied:?}"
    );
}

/// A spinning tool yields: other work on the same runtime thread still runs meanwhile.
#[tokio::test(flavor = "current_thread")]
async fn a_busy_tool_does_not_starve_the_runtime() {
    // Compile first, so the clock below times only the run.
    let _ = js(
        "function main() { return 1; }",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await;
    let started = std::time::Instant::now();
    let spinning = async {
        let result = js(
            "function main() { while (true) {} }",
            "{}",
            Capabilities::default(),
            Limits {
                fuel: u64::MAX,
                timeout: Duration::from_millis(1500),
                ..Limits::default()
            },
        )
        .await;
        (result, started.elapsed())
    };
    let ticker = async {
        for _ in 0..5 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        started.elapsed()
    };
    let ((result, spun), ticked) = tokio::join!(spinning, ticker);
    assert_eq!(result.unwrap_err(), SandboxError::Timeout);
    // The guest kept running for its whole time limit, and on this one thread the ticker
    // still finished long before it: the guest yielded instead of holding the thread.
    assert!(spun >= Duration::from_millis(1400), "{spun:?}");
    assert!(ticked < Duration::from_millis(1000), "{ticked:?}");
}

/// DNS rebinding: the checked address is the one connected to; the name is not looked up again.
#[tokio::test]
async fn the_connection_goes_to_the_checked_address() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let public: SocketAddr = "93.184.216.34:443".parse().unwrap();
    let private: SocketAddr = "10.0.0.5:443".parse().unwrap();
    assert_eq!(checked_address([private, public]), Some(public));
    assert_eq!(checked_address([private]), None);
    // A name that resolves nowhere still reaches the pinned address, so DNS is not asked.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\npinned")
            .await
            .unwrap();
    });
    // A proxy in the environment is not used: it would look the name up again.
    std::env::set_var("HTTP_PROXY", "http://127.0.0.1:9");
    std::env::set_var("http_proxy", "http://127.0.0.1:9");
    let client = pinned_client("rebind.invalid", address).unwrap();
    std::env::remove_var("HTTP_PROXY");
    std::env::remove_var("http_proxy");
    let body = client
        .get(format!("http://rebind.invalid:{}/", address.port()))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "pinned");
    server.await.unwrap();
}

/// The CPU budget: fuel runs out before the clock does.
#[tokio::test]
async fn fuel_bounds_the_cpu() {
    let err = js(
        "function main() { let n = 0; while (true) { n++; } }",
        "{}",
        Capabilities::default(),
        Limits {
            fuel: 50_000_000,
            timeout: Duration::from_secs(60),
            ..Limits::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err, SandboxError::OutOfFuel);
}

/// A hard link in a granted folder to a file in another one is refused, for reads and writes,
/// and the linked file is left as it was.
#[cfg(unix)]
#[tokio::test]
async fn a_hard_link_into_an_ungranted_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::create_dir_all(dir.path().join("private")).unwrap();
    std::fs::write(dir.path().join("private/y.txt"), "secret").unwrap();
    std::fs::hard_link(
        dir.path().join("private/y.txt"),
        dir.path().join("data/x.txt"),
    )
    .unwrap();
    let caps = Capabilities {
        read_prefixes: vec!["data".into()],
        write_prefixes: vec!["data".into()],
        ..Default::default()
    };
    for script in [
        "function main() { return read('data/x.txt'); }",
        "function main() { write('data/x.txt', 'overwritten'); }",
    ] {
        let (out, _) = run(
            script,
            "{}",
            &caps,
            Limits::default(),
            Some(Workspace {
                root: dir.path().to_path_buf(),
                read: |_, bytes| Ok(bytes),
                write: |_, b| Ok(b.to_vec()),
            }),
        )
        .await;
        assert!(out.unwrap_err().to_string().contains("link"), "{script}");
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("private/y.txt")).unwrap(),
        "secret"
    );
}

/// A FIFO in the workspace (which `run_code` can make) is refused at once, not waited on.
#[cfg(unix)]
#[tokio::test]
async fn a_fifo_in_the_workspace_does_not_hang_the_call() {
    let dir = tempfile::tempdir().unwrap();
    let fifo =
        std::ffi::CString::new(dir.path().join("pipe").as_os_str().to_str().unwrap()).unwrap();
    // SAFETY: a valid C string path.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    // Compile first, so the clock below times only the call.
    let _ = js(
        "function main() { return 1; }",
        "{}",
        Capabilities::default(),
        Limits::default(),
    )
    .await;
    let started = std::time::Instant::now();
    let (out, _) = run(
        "function main() { return read('pipe'); }",
        "{}",
        &Capabilities {
            read_prefixes: vec!["".into()],
            ..Default::default()
        },
        Limits {
            timeout: Duration::from_secs(10),
            ..Limits::default()
        },
        Some(Workspace {
            root: dir.path().to_path_buf(),
            read: |_, bytes| Ok(bytes),
            write: |_, b| Ok(b.to_vec()),
        }),
    )
    .await;
    assert!(out.unwrap_err().to_string().contains("not a file"));
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The race: another process swaps a link in and out of the file's place while a tool writes
/// it. The file is opened from the workspace without following links, so nothing ever lands
/// outside, however the timing falls.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_swapped_in_mid_write_is_never_followed() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("victim.txt");
    std::fs::create_dir_all(dir.path().join("out")).unwrap();
    let place = dir.path().join("out/x.txt");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swapper = {
        let (place, target, stop) = (place.clone(), target.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = std::os::unix::fs::symlink(&target, &place);
                let _ = std::fs::remove_file(&place);
            }
        })
    };
    let caps = Capabilities {
        write_prefixes: vec!["out".into()],
        ..Default::default()
    };
    for _ in 0..200 {
        let _ = run(
            "function main() { write('out/x.txt', 'pwned'); }",
            "{}",
            &caps,
            Limits::default(),
            Some(Workspace {
                root: dir.path().to_path_buf(),
                read: |_, bytes| Ok(bytes),
                write: |_, b| Ok(b.to_vec()),
            }),
        )
        .await;
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    swapper.join().unwrap();
    assert!(!target.exists(), "a write went through the link");
}

/// Output handling: reads and writes past 1 MiB are refused.
#[tokio::test]
async fn reads_and_writes_are_capped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("big.txt"), vec![b'x'; (1 << 20) + 1]).unwrap();
    let caps = Capabilities {
        read_prefixes: vec!["".into()],
        write_prefixes: vec!["".into()],
        ..Default::default()
    };
    let workspace = || {
        Some(Workspace {
            root: dir.path().to_path_buf(),
            read: |_, bytes| Ok(bytes),
            write: |_, b| Ok(b.to_vec()),
        })
    };
    let (read, _) = run(
        "function main() { return read('big.txt').length; }",
        "{}",
        &caps,
        Limits::default(),
        workspace(),
    )
    .await;
    assert!(read.unwrap_err().to_string().contains("larger than"));
    let (wrote, _) = run(
        "function main() { write('out.txt', 'x'.repeat(1048577)); }",
        "{}",
        &caps,
        Limits::default(),
        workspace(),
    )
    .await;
    assert!(wrote.unwrap_err().to_string().contains("at most"));
    assert!(!dir.path().join("out.txt").exists());
    // And what a script returns is capped too.
    let (big, _) = run(
        "function main() { return 'x'.repeat(1048577); }",
        "{}",
        &caps,
        Limits::default(),
        None,
    )
    .await;
    assert!(big.unwrap_err().to_string().contains("larger than"));
}

/// A link inside a granted folder that points into another folder of the workspace is
/// refused: the grant is checked where the link leads.
#[cfg(unix)]
#[tokio::test]
async fn a_read_cannot_follow_a_link_into_an_ungranted_folder() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::create_dir_all(dir.path().join("private")).unwrap();
    std::fs::write(dir.path().join("private/x.txt"), "secret").unwrap();
    std::os::unix::fs::symlink("../private/x.txt", dir.path().join("data/x.txt")).unwrap();
    std::fs::write(dir.path().join("data/ok.txt"), "fine").unwrap();
    let read = |file: &str| {
        let script = format!("function main() {{ return read('{file}'); }}");
        let root = dir.path().to_path_buf();
        async move {
            run(
                &script,
                "{}",
                &Capabilities {
                    read_prefixes: vec!["data".into()],
                    ..Default::default()
                },
                Limits::default(),
                Some(Workspace {
                    root,
                    read: |_, bytes| Ok(bytes),
                    write: |_, b| Ok(b.to_vec()),
                }),
            )
            .await
            .0
        }
    };
    assert_eq!(read("data/ok.txt").await.unwrap(), "\"fine\"");
    let err = read("data/x.txt").await.unwrap_err().to_string();
    assert!(err.contains("outside the folders"), "{err}");
}

/// Output handling: a response is cut off as soon as it passes the cap, chunk by chunk.
#[test]
fn a_response_past_the_cap_is_refused_as_it_arrives() {
    let mut body = Vec::new();
    assert!(push_capped(&mut body, &[1; 600], 1000).is_ok());
    let err = push_capped(&mut body, &[1; 600], 1000).unwrap_err();
    assert!(err.contains("larger than 1000 bytes"), "{err}");
    assert_eq!(body.len(), 600, "the chunk past the cap is not kept");
    assert_eq!(MAX_HTTP_BODY_BYTES, 1 << 20);
}

/// A deterministic run sees a fixed clock and seeded randomness: the same arguments give the
/// same result, today and tomorrow. An ordinary run sees the real time.
#[tokio::test]
async fn a_deterministic_run_gives_the_same_result_every_time() {
    let script = "function main() { return [Date.now(), Math.random()]; }";
    let fixed = Limits {
        deterministic: true,
        ..Limits::default()
    };
    let first = js(script, "{}", Capabilities::default(), fixed)
        .await
        .unwrap();
    let second = js(script, "{}", Capabilities::default(), fixed)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with("[1767225600000,"), "{first}");
    let real = js(script, "{}", Capabilities::default(), Limits::default())
        .await
        .unwrap();
    assert!(!real.starts_with("[1767225600000,"), "{real}");
}

/// Reads cannot leave the workspace through a link.
#[cfg(unix)]
#[tokio::test]
async fn a_read_cannot_follow_a_link_out_of_the_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "no").unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
    let (out, _) = run(
        "function main() { return read('link/secret.txt'); }",
        "{}",
        &Capabilities {
            read_prefixes: vec!["".into()],
            ..Default::default()
        },
        Limits::default(),
        Some(Workspace {
            root: dir.path().to_path_buf(),
            read: |_, bytes| Ok(bytes),
            write: |_, b| Ok(b.to_vec()),
        }),
    )
    .await;
    let err = out.unwrap_err().to_string();
    assert!(err.contains("leaves the workspace"), "{err}");
}

/// Credentials: only for the host they were granted for, and only when the value is known.
#[test]
fn a_credential_goes_only_to_its_host() {
    let caps = Capabilities {
        http_hosts: vec!["api.example.com".into(), "other.example.com".into()],
        credentials: vec!["api.example.com=EXAMPLE_TOKEN".into()],
        ..Default::default()
    };
    let secrets: std::collections::HashMap<String, String> =
        [("EXAMPLE_TOKEN".to_string(), "s3cret-value".to_string())].into();
    assert_eq!(
        credential_for(&caps, &secrets, "API.example.com"),
        Some("s3cret-value")
    );
    assert_eq!(credential_for(&caps, &secrets, "other.example.com"), None);
    assert_eq!(
        credential_for(&caps, &Default::default(), "api.example.com"),
        None
    );
    // Attached as a bearer token on the request itself.
    let request = pinned_client("api.example.com", "93.184.216.34:443".parse().unwrap())
        .unwrap()
        .get("https://api.example.com/")
        .bearer_auth(credential_for(&caps, &secrets, "api.example.com").unwrap())
        .build()
        .unwrap();
    assert_eq!(request.headers()["authorization"], "Bearer s3cret-value");
}

#[test]
fn private_addresses_are_recognized() {
    for ip in [
        "10.0.0.1",
        "192.168.1.1",
        "127.0.0.1",
        "169.254.169.254",
        "100.64.0.1",
        "::1",
        "fd00::1",
        "fe80::1",
        "::ffff:10.0.0.1",
        // Ranges that are not on the internet, and IPv4 inside IPv6.
        "0.1.2.3",
        "224.0.0.1",
        "198.18.0.1",
        "240.0.0.1",
        "192.0.0.8",
        "ff02::1",
        "::10.0.0.1",
        "64:ff9b::a00:1",
        "2002:a00:1::",
        "2001:0:4136:e378:8000:63bf:3fff:fdd2",
        "64:ff9b:1::a00:1",
        "2001:db8::1",
    ] {
        assert!(is_private(ip.parse().unwrap()), "{ip}");
    }
    for ip in [
        "1.1.1.1",
        "93.184.216.34",
        "2606:4700::1111",
        "64:ff9b::5db8:d822",
        "2002:5db8:d822::",
    ] {
        assert!(!is_private(ip.parse().unwrap()), "{ip}");
    }
}
