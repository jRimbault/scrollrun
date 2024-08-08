#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
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
#[derive(Parser)]
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

const DELAY: std::time::Duration = std::time::Duration::from_millis(100);

impl Opt {
    fn num_lines(&self) -> Option<usize> {
        if let Some(i) = self.num_lines {
            return Some(i);
        }
        termsize::get().map(|t| t.rows.saturating_sub(30).max(10) as usize)
    }
}

fn main() -> anyhow::Result<ExitCode> {
    let opt = Opt::parse();
    let num_lines = opt.num_lines().unwrap_or(10);
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
            let stderr = s.spawn(move || print(receiver, num_lines));
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
            let h = s.spawn(move || print(receiver, num_lines));
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

fn print(rx: mpsc::Receiver<String>, num_lines: usize) {
    let start = Instant::now();
    let mut output_lines = VecDeque::new();
    let mut has_ended = false;
    loop {
        while let Ok(line) = rx.try_recv() {
            output_lines.push_back(line);
        }
        print!("\x1B[2J\x1B[H"); // clear
        println!("· Elapsed time: {}", Duration(start.elapsed()));
        println!("╭─");
        for line in output_lines.iter().take(num_lines) {
            println!("│ {line}");
        }
        println!("╰─");
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
    println!("· Finished in: {}", Duration(start.elapsed()));
}

#[derive(Debug)]
struct Duration(std::time::Duration);

impl fmt::Display for Duration {
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
