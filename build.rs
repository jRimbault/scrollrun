fn main() {
    let version = git_version().unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap());
    println!(
        "cargo:rustc-env=PKG_VERSION={}",
        &version[..version.len().min(8)]
    );
    println!("cargo:rustc-env=PKG_LONG_VERSION={}", version);
}

fn git_version() -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["show", "-s", "--format=%H"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output = String::from_utf8(output.stdout).ok()?;
    Some(output)
}
