# scrollrun

Display a command output in a fixed amount of lines with the elasped time.

## Usage

Example:

```sh
scrollrun "while true; do openssl rand -base64 10; sleep .5; done"
```

Output:

```text
· Elapsed time: 00:00:08
╭─
│ wfEaPaVO+KhQkw==
│ AD/MzXsaIDgMdw==
│ MpwSyWp+YHPImA==
│ vmWzGqGmNaqs2A==
│ NR1FqlCTEPkBfw==
│ JWyhIqQTqu7LJg==
│ lIXRBmqecXLqrQ==
│ 5j02LbKvmewtxw==
│ gpIVuVsBRRSpqQ==
│ cdbrzcRFB5W0dQ==
╰─
```

<details>
<summary>Help</summary>


```text
Usage: scrollrun [OPTIONS] <COMMAND>

Run a command and display its output in a scrolling window. Doesn't particularly work well with commands outputing control characters

Arguments:
  <COMMAND>  The command to run. Will be run through a shell.

Options:
  -n, --num-lines <NUM_LINES>  Number of lines to display at a time [default: 10]
  -h, --help                   Print help (see more with '--help')
  -V, --version                Print version

scrollrun 0.1.0 jRimbault <jacques.rimbault@gmail.com>
```

</details>

## Install

```sh
cargo install --locked https://github.com/jRimbault/scrollrun.git
```
