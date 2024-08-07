use clap::Parser;
use std::{
    process::{Command, ExitCode, Stdio},
    sync::mpsc::{self},
    thread,
    time::{Duration, Instant},
};

/// Run a command and display its output in a scrolling window.
/// Doesn't particularly work well with outputs with lots of control characters.
#[derive(Parser)]
struct Opt {
    /// The command to run
    command: String,
    /// Number of lines to display at a time
    #[clap(short, long, default_value = "10")]
    num_lines: usize,
}

#[derive(Debug, Clone, Copy)]
enum Source {
    Stdout,
    Stderr,
}

fn clear() {
    print!("\x1B[2J\x1B[H");
}

const DELAY: Duration = Duration::from_millis(100);

fn main() -> anyhow::Result<ExitCode> {
    let opt = Opt::parse();

    let (tx, rx) = mpsc::channel();
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(&opt.command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout = consumer::Consumer::new(stdout, tx.clone());
    let stderr = consumer::Consumer::new(stderr, tx.clone());

    let start = Instant::now();
    let mut output_lines = Vec::new();

    loop {
        while let Ok(line) = rx.try_recv() {
            output_lines.push(line);
        }

        clear();
        println!(
            "· Elapsed time: {}",
            indicatif::FormattedDuration(start.elapsed())
        );
        println!("╭─");
        for (line, source) in output_lines.iter().rev().take(opt.num_lines).rev() {
            match source {
                Source::Stdout => println!("│ {line}"),
                Source::Stderr => eprintln!("│ {line}"),
            }
        }
        println!("╰─");

        if output_lines.len() > opt.num_lines {
            output_lines.drain(..output_lines.len() - opt.num_lines);
        }

        if let Some(status) = child.try_wait()? {
            if let Err(mpsc::RecvTimeoutError::Timeout) = rx.recv_timeout(DELAY) {
                stdout.close()?;
                stderr.close()?;
                println!(
                    "· Finished in: {}",
                    indicatif::FormattedDuration(start.elapsed())
                );
                let code = status
                    .code()
                    .and_then(|i| u8::try_from(i).ok())
                    .unwrap_or_else(|| if status.success() { 0 } else { 1 });
                return Ok(ExitCode::from(code));
            }
        }

        thread::sleep(DELAY);
    }
}

mod consumer {
    use std::{
        io::{BufRead, BufReader, Read},
        sync::mpsc::Sender,
        thread,
    };

    use crate::Source;

    #[derive(Debug)]
    pub struct Consumer {
        handle: Option<thread::JoinHandle<()>>,
    }

    impl Consumer {
        pub fn new<R>(reader: R, tx: Sender<(String, Source)>) -> Self
        where
            R: MarkedReader + 'static,
        {
            let reader = BufReader::new(reader);
            let handle = thread::spawn(move || {
                for line in reader.lines() {
                    match line {
                        Ok(line) => tx.send((line, R::MARKER)).unwrap(),
                        Err(_) => break,
                    }
                }
            });

            Self {
                handle: Some(handle),
            }
        }
        pub fn close(mut self) -> anyhow::Result<()> {
            if let Some(handle) = self.handle.take() {
                handle
                    .join()
                    .map_err(|_| anyhow::anyhow!("couldn't join thread"))?;
            }
            Ok(())
        }
    }

    impl Drop for Consumer {
        fn drop(&mut self) {
            if let Some(handle) = self.handle.take() {
                let _ = handle.join();
            }
        }
    }

    trait Sealed {}

    #[allow(private_bounds)]
    pub trait MarkedReader: Read + Send + Sized + Sealed {
        const MARKER: Source;
    }

    impl Sealed for std::process::ChildStdout {}
    impl Sealed for std::process::ChildStderr {}

    impl MarkedReader for std::process::ChildStdout {
        const MARKER: Source = Source::Stdout;
    }

    impl MarkedReader for std::process::ChildStderr {
        const MARKER: Source = Source::Stderr;
    }
}
