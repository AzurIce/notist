//! Experimental JSON transport for native command extensions.
//! Protocol stages and payload validation belong to each extension adapter.
use serde::{Serialize, de::DeserializeOwned};
use std::{
    io::{self, Write},
    path::Path,
    process::{Command, Stdio},
};

/// Send one JSON value and EOF, drain stdout, and decode one response.
/// Arguments do not pass through a shell. Stderr inherits the host's stream.
pub fn exchange<I: Serialize, O: DeserializeOwned>(
    command: &[String],
    directory: &Path,
    request: &I,
) -> io::Result<O> {
    let Some(program) = command.first().filter(|s| !s.is_empty()) else {
        return Err(io::Error::other("command requires a nonempty program"));
    };
    let input = serde_json::to_vec(request)
        .map_err(|e| io::Error::other(format!("cannot encode request: {e}")))?;
    let mut child = Command::new(program)
        .args(&command[1..])
        .current_dir(directory)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| io::Error::other(format!("cannot start command: {e}")))?;
    let mut stdin = child.stdin.take().unwrap();
    // Drain stdout while writing stdin so neither pipe can block the other.
    let (output, written) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(&input));
        (child.wait_with_output(), writer.join())
    });
    let output = output.map_err(|e| io::Error::other(format!("cannot wait for command: {e}")))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "command exited with {}",
            output.status
        )));
    }
    written
        .map_err(|_| io::Error::other("request writer failed"))?
        .map_err(|e| io::Error::other(format!("cannot write request: {e}")))?;
    serde_json::from_slice(&output.stdout)
        .map_err(|e| io::Error::other(format!("invalid JSON response: {e}")))
}
