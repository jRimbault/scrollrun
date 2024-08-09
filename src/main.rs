#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(
    bad_style,
    dead_code,
    improper_ctypes,
    missing_debug_implementations,
    non_shorthand_field_patterns,
    no_mangle_generic_items,
    overflowing_literals,
    path_statements,
    patterns_in_fns_without_body,
    unconditional_recursion,
    unused,
    unused_allocation,
    unused_comparisons,
    unused_parens,
    while_true
)]
#![cfg_attr(
    feature = "more-warnings",
    warn(
        trivial_casts,
        trivial_numeric_casts,
        unused_extern_crates,
        unused_import_braces,
        unused_qualifications,
    )
)]
#![cfg_attr(
    feature = "even-more-warnings",
    warn(missing_copy_implementations, missing_docs)
)]

use clap::Parser;
use std::{
    collections::VecDeque,
    fmt,
    io::{BufRead, BufReader, IsTerminal},
    process::{Command, ExitCode, Stdio},
    sync::mpsc::{self},
    thread,
    time::Instant,
};

/// Run a command and display its output in a scrolling window.
/// Doesn't particularly work well with commands outputing control characters.
#[derive(Debug, Parser)]
#[clap(
    version,
    author = clap::crate_authors!("\n"),
    styles = styles(),
    help_template = HELP,
)]
struct Opt {
    /// The command to run. Will be run through a shell.
    command: Option<String>,
    /// Number of lines to display at a time
    #[clap(short, long)]
    num_lines: Option<usize>,
}

impl Opt {
    fn num_lines(&self) -> Option<usize> {
        static CALLED: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
        static ROWS: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);
        if let Some(i) = self.num_lines {
            return Some(i);
        }
        let rows = if CALLED.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            let term = termsize::get()?;
            ROWS.store(term.rows, std::sync::atomic::Ordering::SeqCst);
            term.rows
        } else {
            ROWS.load(std::sync::atomic::Ordering::SeqCst)
        };
        if CALLED.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 10 {
            CALLED.store(0, std::sync::atomic::Ordering::SeqCst);
        }
        let rows = rows.saturating_sub(20).max(10).min(rows);
        Some(rows.into())
    }
}

fn main() -> anyhow::Result<ExitCode> {
    let opt = Opt::parse();
    let code = thread::scope(|s| -> anyhow::Result<_> {
        let (sender, receiver) = mpsc::channel();
        if let Some(cmd) = &opt.command {
            let mut child = Command::new("bash")
                .arg("--norc")
                .arg("-c")
                .arg(cmd)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?;
            s.spawn({
                let stdout = child.stdout.take().unwrap();
                let sender = sender.clone();
                move || read(stdout, sender)
            });
            let stdout = s.spawn({
                let stderr = child.stderr.take().unwrap();
                move || read(stderr, sender)
            });
            let stderr = s.spawn(move || {
                let stdout = std::io::stdout();
                print(stdout.lock(), receiver, opt);
            });
            let status = child.wait()?;
            let _ = stdout.join();
            let _ = stderr.join();
            return Ok(status
                .code()
                .and_then(|i| u8::try_from(i).ok())
                .map(ExitCode::from)
                .unwrap_or(if status.success() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }));
        }
        if !std::io::stdin().is_terminal() {
            let stdin = s.spawn(move || read(std::io::stdin(), sender));
            let h = s.spawn(move || {
                let stdout = std::io::stdout();
                print(stdout.lock(), receiver, opt)
            });
            let _ = stdin.join();
            h.join()
                .map_err(|_| anyhow::anyhow!("couldn't read from pipe"))?;
        }
        Ok(ExitCode::SUCCESS)
    })?;
    Ok(code)
}

fn read<R>(reader: R, tx: mpsc::Sender<String>)
where
    R: std::io::Read,
{
    let stdout = BufReader::new(reader);
    for line in stdout.lines() {
        match line {
            Ok(line) => tx.send(line).unwrap(),
            Err(_) => break,
        }
    }
}

fn print<W>(mut writer: W, rx: mpsc::Receiver<String>, opt: Opt)
where
    W: std::io::Write,
{
    const DELAY: std::time::Duration = std::time::Duration::from_millis(100);
    let start = Instant::now();
    let mut output_lines = VecDeque::new();
    let mut has_ended = false;
    loop {
        let num_lines = opt.num_lines().unwrap_or(10);
        while let Ok(line) = rx.try_recv() {
            output_lines.push_back(line);
        }
        writeln!(writer, "\x1B[2J\x1B[H").unwrap(); // clear
        writeln!(writer, "· Elapsed time: {}", Format(start.elapsed())).unwrap();
        writeln!(writer, "╭─").unwrap();
        for line in output_lines.iter().take(num_lines) {
            writeln!(writer, "│ {line}").unwrap();
        }
        writeln!(writer, "╰─").unwrap();
        while output_lines.len() > num_lines {
            output_lines.pop_front();
        }
        match rx.recv_timeout(DELAY) {
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if has_ended {
                    break;
                }
                has_ended = true;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Ok(line) => output_lines.push_back(line),
        }
        thread::sleep(DELAY);
    }
    writeln!(writer, "· Finished in: {}", Format(start.elapsed())).unwrap();
}

#[derive(Debug)]
struct Format(std::time::Duration);

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut t = self.0.as_secs();
        let seconds = t % 60;
        t /= 60;
        let minutes = t % 60;
        t /= 60;
        let hours = t % 24;
        t /= 24;
        if t > 0 {
            let days = t;
            write!(f, "{days}d {hours:02}:{minutes:02}:{seconds:02}")
        } else {
            write!(f, "{hours:02}:{minutes:02}:{seconds:02}")
        }
    }
}

const HELP: &str = "\
{before-help}
{usage-heading} {usage}

{about}

{all-args}{after-help}

{name} {version} {author-with-newline}
";

fn styles() -> clap::builder::Styles {
    use clap::builder::styling::AnsiColor;
    clap::builder::Styles::styled()
        .usage(AnsiColor::Green.on_default())
        .header(AnsiColor::Yellow.on_default())
        .literal(AnsiColor::Green.on_default())
        .placeholder(AnsiColor::Green.on_default())
}

#[cfg(test)]
mod test {
    use super::{print, read, Format, Opt};

    use std::{io::Cursor, sync::mpsc, thread, time::Duration};

    #[test]
    fn single_line() {
        let input = "This is a single line";
        let reader = Cursor::new(input);
        let (tx, rx) = mpsc::channel();

        read(reader, tx);

        let result: Vec<String> = rx.iter().collect();
        assert_eq!(result, vec!["This is a single line"]);
    }

    #[test]
    fn multiple_lines() {
        let input = "Line 1\nLine 2\nLine 3\n";
        let reader = Cursor::new(input);
        let (tx, rx) = mpsc::channel();

        read(reader, tx);

        let result: Vec<String> = rx.iter().collect();
        assert_eq!(result, vec!["Line 1", "Line 2", "Line 3"]);
    }

    #[test]
    fn empty_input() {
        let input = "";
        let reader = Cursor::new(input);
        let (tx, rx) = mpsc::channel();

        read(reader, tx);

        let result: Vec<String> = rx.iter().collect();
        assert!(result.is_empty());
    }

    #[test]
    #[should_panic]
    fn lines_with_errors() {
        let input = "Line 1\nLine 2\nLine 3\n";
        let reader = Cursor::new(input);
        let (tx, rx) = mpsc::channel();

        // Simulate an error by dropping the receiver in another thread
        let handle = thread::spawn(move || {
            read(reader, tx);
        });

        // Drop the receiver to cause the sender to fail
        drop(rx);
        handle.join().unwrap();
    }

    #[test]
    fn mixed_newlines() {
        let input = "Line 1\r\nLine 2\nLine 3\r\n";
        let reader = Cursor::new(input);
        let (tx, rx) = mpsc::channel();

        read(reader, tx);

        let result: Vec<String> = rx.iter().collect();
        assert_eq!(result, vec!["Line 1", "Line 2", "Line 3"]);
    }

    #[test]
    fn print_basic() {
        let (tx, rx) = mpsc::channel();
        let mut output = Cursor::new(Vec::new());

        tx.send("Line 1".to_string()).unwrap();
        tx.send("Line 2".to_string()).unwrap();
        drop(tx);

        print(
            &mut output,
            rx,
            Opt {
                command: None,
                num_lines: Some(5),
            },
        );

        let output_str = String::from_utf8(output.into_inner()).unwrap();
        assert!(output_str.contains("Line 1"));
        assert!(output_str.contains("Line 2"));
        assert!(output_str.contains("· Finished in:"));
    }

    #[test]
    fn print_with_more_lines_than_display() {
        let (tx, rx) = mpsc::channel();
        let mut output = Cursor::new(Vec::new());

        for i in 1..10 {
            tx.send(format!("Line {}", i)).unwrap();
        }
        drop(tx);

        print(
            &mut output,
            rx,
            Opt {
                command: None,
                num_lines: Some(5),
            },
        );

        let output_str = String::from_utf8(output.into_inner()).unwrap();
        for i in 1..10 {
            assert!(output_str.contains(&format!("Line {}", i)));
        }
        assert!(output_str.contains("· Finished in:"));
    }

    #[test]
    fn print_timeout() {
        let (tx, rx) = mpsc::channel();
        let mut output = Cursor::new(Vec::new());

        tx.send("Line 1".to_string()).unwrap();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            tx.send("Line 2".to_string()).unwrap();
        });

        print(
            &mut output,
            rx,
            Opt {
                command: None,
                num_lines: Some(5),
            },
        );

        let output_str = String::from_utf8(output.into_inner()).unwrap();
        assert!(output_str.contains("Line 1"));
        assert!(output_str.contains("Line 2"));
        assert!(output_str.contains("· Finished in:"));
    }

    #[test]
    fn format_seconds() {
        let duration = Format(Duration::from_secs(45));
        let formatted = format!("{}", duration);
        assert_eq!(formatted, "00:00:45");
    }

    #[test]
    fn format_minutes() {
        let duration = Format(Duration::from_secs(125));
        let formatted = format!("{}", duration);
        assert_eq!(formatted, "00:02:05");
    }

    #[test]
    fn format_hours() {
        let duration = Format(Duration::from_secs(3665));
        let formatted = format!("{}", duration);
        assert_eq!(formatted, "01:01:05");
    }

    #[test]
    fn format_days() {
        let duration = Format(Duration::from_secs(90065));
        let formatted = format!("{}", duration);
        assert_eq!(formatted, "1d 01:01:05");
    }

    #[test]
    fn format_multiple_days() {
        let duration = Format(Duration::from_secs(200065));
        let formatted = format!("{}", duration);
        assert_eq!(formatted, "2d 07:34:25");
    }

    #[test]
    fn format_edge_case() {
        let duration = Format(Duration::from_secs(86400)); // Exactly one day
        let formatted = format!("{}", duration);
        assert_eq!(formatted, "1d 00:00:00");
    }
}
