use std::{
    io::{self, Read},
    process::{Command, ExitStatus, Stdio},
    thread,
};

#[derive(Debug)]
pub struct BoundedCommandOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// Runs a child while draining both output pipes concurrently and retaining at
/// most the configured bytes from each stream. Draining beyond the retained
/// limit prevents a verbose child from deadlocking on a full pipe without
/// allowing its output to grow Axiom's memory without bound.
pub fn run_command_bounded(
    command: &mut Command,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> io::Result<BoundedCommandOutput> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("bounded child process did not expose a stdout pipe"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("bounded child process did not expose a stderr pipe"))?;

    let stdout_reader = thread::spawn(move || read_bounded(stdout, max_stdout_bytes));
    let stderr_reader = thread::spawn(move || read_bounded(stderr, max_stderr_bytes));
    let status = child.wait()?;
    let (stdout, stdout_truncated) = stdout_reader
        .join()
        .map_err(|_| io::Error::other("stdout reader thread terminated unexpectedly"))??;
    let (stderr, stderr_truncated) = stderr_reader
        .join()
        .map_err(|_| io::Error::other("stderr reader thread terminated unexpectedly"))??;

    Ok(BoundedCommandOutput {
        status,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
    })
}

fn read_bounded(mut reader: impl Read, limit: usize) -> io::Result<(Vec<u8>, bool)> {
    let mut retained = Vec::with_capacity(limit.min(64 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(retained.len());
        let keep = read.min(remaining);
        retained.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    Ok((retained, truncated))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn bounded_reader_retains_limit_and_drains_remainder() {
        let (retained, truncated) =
            read_bounded(Cursor::new(b"abcdefgh"), 3).expect("bounded read");

        assert_eq!(retained, b"abc");
        assert!(truncated);
    }

    #[test]
    fn bounded_reader_reports_complete_short_stream() {
        let (retained, truncated) = read_bounded(Cursor::new(b"abc"), 8).expect("bounded read");

        assert_eq!(retained, b"abc");
        assert!(!truncated);
    }
}
