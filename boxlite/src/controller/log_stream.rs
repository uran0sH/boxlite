//! Log streaming from subprocess stdout/stderr to parent's tracing system.

use std::{
    io::{BufRead, BufReader},
    process::{ChildStderr, ChildStdout},
    thread::{self, JoinHandle},
};

use boxlite_shared::errors::{BoxliteError, BoxliteResult};

/// Log level for subprocess output streams.
#[derive(Debug, Clone, Copy)]
pub(super) enum LogLevel {
    Debug,
    Warn,
}

/// Handles log streaming from subprocess stdout/stderr to parent's tracing system.
///
/// This type owns the reader threads and ensures they are properly joined
/// when the subprocess exits. Each stream (stdout/stderr) gets a dedicated
/// thread that reads lines and logs them with appropriate levels.
pub(super) struct LogStreamHandler {
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl LogStreamHandler {
    /// Creates a new log stream handler by spawning reader threads for stdout and stderr.
    ///
    /// # Arguments
    /// * `stdout` - Subprocess stdout pipe
    /// * `stderr` - Subprocess stderr pipe
    ///
    /// # Returns
    /// * `Ok(LogStreamHandler)` - Successfully spawned both reader threads
    /// * `Err(...)` - Failed to spawn one or both threads
    pub(super) fn new(stdout: ChildStdout, stderr: ChildStderr) -> BoxliteResult<Self> {
        // Spawn stdout reader thread (DEBUG level for informational logs)
        let stdout_thread =
            Self::spawn_reader_thread(BufReader::new(stdout), "stdout", LogLevel::Debug)?;

        // Spawn stderr reader thread (WARN level for errors/warnings)
        let stderr_thread =
            Self::spawn_reader_thread(BufReader::new(stderr), "stderr", LogLevel::Warn)?;

        Ok(Self {
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
        })
    }

    /// Strips ANSI escape codes from a string.
    ///
    /// Subprocess logs may contain ANSI color codes from tracing formatters.
    /// We strip these before re-logging to avoid double-formatting.
    fn strip_ansi_codes(text: &str) -> String {
        // Simple ANSI escape sequence pattern: \x1b[...m
        let mut result = String::with_capacity(text.len());
        let mut chars = text.chars();

        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Skip ANSI escape sequence
                if chars.next() == Some('[') {
                    // Skip until 'm' or end of string
                    for next_char in chars.by_ref() {
                        if next_char == 'm' {
                            break;
                        }
                    }
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    /// Spawns a thread that reads from a pipe and logs each line.
    ///
    /// The thread runs until the pipe is closed (subprocess exits) or an I/O error occurs.
    /// Each line is logged with the "boxlite-shim" target for easy filtering.
    /// ANSI escape codes are stripped to prevent double-formatting.
    ///
    /// # Arguments
    /// * `reader` - Buffered reader wrapping the pipe
    /// * `stream_name` - Name of the stream for thread naming ("stdout" or "stderr")
    /// * `log_level` - Log level to use (Debug for stdout, Warn for stderr)
    ///
    /// # Returns
    /// * `Ok(JoinHandle)` - Successfully spawned thread
    /// * `Err(...)` - Failed to spawn thread
    fn spawn_reader_thread<R: BufRead + Send + 'static>(
        reader: R,
        stream_name: &str,
        log_level: LogLevel,
    ) -> BoxliteResult<JoinHandle<()>> {
        let thread_name = format!("boxlite-shim-{}", stream_name);
        let stream_name_owned = stream_name.to_string();

        thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            // Strip ANSI codes from subprocess output to avoid double-formatting
                            let clean_line = Self::strip_ansi_codes(&line);

                            match log_level {
                                LogLevel::Debug => {
                                    tracing::debug!(target: "box:stdout", "{}", clean_line);
                                }
                                LogLevel::Warn => {
                                    tracing::warn!(target: "box:stderr", "{}", clean_line);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                target: "box:stdout/stderr",
                                stream = %stream_name_owned,
                                "Failed to read from pipe: {}", e
                            );
                            break;
                        }
                    }
                }
                tracing::debug!(
                    target: "box:stdout/stderr",
                    stream = %stream_name_owned,
                    "Pipe closed, thread exiting"
                );
            })
            .map_err(|e| {
                BoxliteError::Engine(format!(
                    "Failed to spawn {} reader thread: {}",
                    stream_name, e
                ))
            })
    }

    /// Gracefully shuts down log streaming by waiting for reader threads to finish.
    ///
    /// This should be called after the subprocess has been killed and pipes are closed.
    /// The threads will exit naturally when they reach EOF on their pipes.
    ///
    /// # Returns
    /// * `Ok(())` - All threads finished successfully
    /// * `Err(...)` - One or more threads panicked (logged as warning)
    pub(super) fn shutdown(mut self) -> BoxliteResult<()> {
        // Wait for stdout thread to finish
        if let Some(handle) = self.stdout_thread.take()
            && let Err(e) = handle.join()
        {
            tracing::warn!(
                target: "boxlite-shim:stdout",
                "stdout reader thread panicked: {:?}", e
            );
        }

        // Wait for stderr thread to finish
        if let Some(handle) = self.stderr_thread.take()
            && let Err(e) = handle.join()
        {
            tracing::warn!(
                target: "boxlite-shim:stderr",
                "stderr reader thread panicked: {:?}", e
            );
        }

        Ok(())
    }
}

impl Drop for LogStreamHandler {
    /// Ensures threads are joined when handler is dropped.
    ///
    /// This is a safety net in case shutdown() wasn't called explicitly.
    fn drop(&mut self) {
        if let Some(handle) = self.stdout_thread.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.stderr_thread.take() {
            let _ = handle.join();
        }
    }
}
