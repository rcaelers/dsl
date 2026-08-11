//! Child-process execution with per-run wall, CPU, and peak-resident-memory accounting.

use std::io::{self, Read};
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::Instant;

pub(crate) struct ProcessOutput {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) wall_seconds: f64,
    pub(crate) cpu_seconds: f64,
    pub(crate) peak_rss_bytes: u64,
}

#[cfg(unix)]
pub(crate) fn execute(
    binary: &Path,
    graph: &Path,
    working_directory: &Path,
) -> io::Result<ProcessOutput> {
    let started = Instant::now();
    let mut child = Command::new(binary)
        .args([
            "run",
            graph.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "graph path is not UTF-8")
            })?,
            "--json",
            "--progress-interval",
            "0",
        ])
        .current_dir(working_directory)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = read_pipe(child.stdout.take().expect("configured stdout pipe"));
    let stderr = read_pipe(child.stderr.take().expect("configured stderr pipe"));
    let mut status = 0;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let pid = i32::try_from(child.id())
        .map_err(|_| io::Error::other("child process identifier exceeds pid_t"))?;
    loop {
        // SAFETY: `pid` is the live child created above, `status` and `usage` are valid writable
        // storage, and no other code waits for this child.
        let waited = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
        if waited == pid {
            break;
        }
        if waited == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful `wait4` initialized the complete `rusage` record.
    let usage = unsafe { usage.assume_init() };
    let stdout = stdout
        .join()
        .map_err(|_| io::Error::other("stdout reader panicked"))??;
    let stderr = stderr
        .join()
        .map_err(|_| io::Error::other("stderr reader panicked"))??;
    drop(child);
    Ok(ProcessOutput {
        status: ExitStatus::from_raw(status),
        stdout,
        stderr,
        wall_seconds: started.elapsed().as_secs_f64(),
        cpu_seconds: timeval_seconds(usage.ru_utime) + timeval_seconds(usage.ru_stime),
        peak_rss_bytes: peak_rss_bytes(usage.ru_maxrss),
    })
}

#[cfg(not(unix))]
pub(crate) fn execute(
    _binary: &Path,
    _graph: &Path,
    _working_directory: &Path,
) -> io::Result<ProcessOutput> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "per-process CPU and peak-RSS accounting is currently supported on Unix hosts",
    ))
}

fn read_pipe(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.read_to_end(&mut bytes)?;
        Ok(bytes)
    })
}

#[cfg(unix)]
fn timeval_seconds(value: libc::timeval) -> f64 {
    value.tv_sec as f64 + value.tv_usec as f64 / 1_000_000.0
}

#[cfg(all(unix, target_os = "macos"))]
fn peak_rss_bytes(value: libc::c_long) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn peak_rss_bytes(value: libc::c_long) -> u64 {
    u64::try_from(value)
        .unwrap_or_default()
        .saturating_mul(1024)
}

#[cfg(all(test, unix))]
mod process_tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn unix_execution_collects_output_cpu_and_peak_memory() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("fixture.sh");
        std::fs::write(&script, "#!/bin/sh\nprintf '{\"ok\":true}'\n").unwrap();
        let mut permissions = script.metadata().unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script, permissions).unwrap();

        let output = execute(&script, Path::new("graph.json"), directory.path()).unwrap();

        assert!(output.status.success());
        assert_eq!(output.stdout, br#"{"ok":true}"#);
        assert!(output.wall_seconds >= 0.0);
        assert!(output.cpu_seconds >= 0.0);
        assert!(output.peak_rss_bytes > 0);
    }
}
