#![doc = include_str!("../README.md")]

use clap::{CommandFactory, Parser};
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
    version = env!("PKG_VERSION"),
    long_version = env!("PKG_LONG_VERSION"),
    author = clap::crate_authors!("\n"),
    styles = clap_cargo::style::CLAP_STYLING,
    help_template = HELP,
)]
struct Opt {
    /// The command to run. Will be run through a shell.
    #[clap(value_hint = clap::ValueHint::CommandString)]
    command: Option<String>,
    /// Number of lines to display at a time
    #[clap(short, long)]
    num_lines: Option<usize>,
    /// Print autocompletion script for your shell
    #[arg(long = "generate", value_enum)]
    generator: Option<clap_complete::Shell>,
}

impl Opt {
    fn num_lines(&self) -> Option<usize> {
        use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
        static CALLED: AtomicU8 = AtomicU8::new(0);
        static ROWS: AtomicU16 = AtomicU16::new(0);
        if let Some(i) = self.num_lines {
            return Some(i);
        }
        let rows = if CALLED.load(Ordering::Relaxed) == 0 {
            let term = termsize::get()?;
            ROWS.store(term.rows, Ordering::Relaxed);
            term.rows
        } else {
            ROWS.load(Ordering::Relaxed)
        };
        if CALLED.fetch_add(1, Ordering::Relaxed) == 10 {
            CALLED.store(0, Ordering::Relaxed);
        }
        Some(num_lines_heuristic(rows).into())
    }
}

fn num_lines_heuristic(rows: u16) -> u16 {
    if rows < 11 {
        rows.saturating_sub(4).max(1)
    } else {
        rows.saturating_sub((rows / 3).max(5)) // Subtract 1/3th or at least 5
    }
}

fn main() -> anyhow::Result<ExitCode> {
    let opt = Opt::parse();
    if print_completions(opt.generator) {
        return Ok(ExitCode::SUCCESS);
    }
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
        write!(writer, "\x1B[2J\x1B[H").unwrap(); // clear
        #[cfg(debug_assertions)]
        write!(writer, "num lines: {num_lines:?} ").unwrap();
        writeln!(writer, "· Elapsed time: {}", Format(start.elapsed())).unwrap();
        writeln!(writer, "╭─").unwrap();
        for line in output_lines.iter().take(num_lines) {
            writeln!(writer, "│ {line}").unwrap();
        }
        writeln!(writer, "╰─").unwrap();
        while output_lines.len() > num_lines {
            has_ended = false;
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

fn print_completions(gen: Option<clap_complete::Shell>) -> bool {
    if let Some(gen) = gen {
        let mut cmd = Opt::command();
        let name = env!("CARGO_BIN_NAME").to_owned();
        clap_complete::generate(gen, &mut cmd, name, &mut std::io::stdout());
        true
    } else {
        false
    }
}

#[cfg(test)]
mod test {
    use textplots::Plot;

    use super::{num_lines_heuristic, print, read, Format, Opt};

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
                generator: None,
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
                generator: None,
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
                generator: None,
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

    #[test]
    fn heuristic_tests() {
        let points: Vec<_> = (0..200).map(num_lines_heuristic).collect();
        if let Some(term) = termsize::get().filter(|t| t.cols > 31 && t.rows > 2) {
            let plot: Vec<(f32, f32)> = points
                .iter()
                .enumerate()
                .map(|(i, p)| (i as _, *p as _))
                .collect();
            println!("{:#?}", plot);
            // should be a quasi linear function
            textplots::Chart::new(term.cols as _, term.cols as _, 0., 200.)
                .lineplot(&textplots::Shape::Lines(&plot))
                .display();
        }
        // ensure that for every increase in terminal row size the window size
        // is at least bigger or equal than for the previous terminal row size
        // assert!(points.is_sorted());
        for w in points.windows(2) {
            assert!(w[0] <= w[1])
        }
    }
}
