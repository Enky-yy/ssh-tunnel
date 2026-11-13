use std::io::{self, IsTerminal, Write};

pub fn colors() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn paint(code: &str, s: &str) -> String {
    if colors() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn green(s: &str) -> String {
    paint("32", s)
}

pub fn red(s: &str) -> String {
    paint("31", s)
}

pub fn yellow(s: &str) -> String {
    paint("33", s)
}

pub fn bold(s: &str) -> String {
    paint("1", s)
}

pub fn dim(s: &str) -> String {
    paint("2", s)
}

pub fn ok() -> String {
    green("✓")
}

pub fn warn() -> String {
    yellow("!")
}

pub fn fail() -> String {
    red("✗")
}

/// Ask a question on stdin, returning `default` when the user submits an empty line.
pub fn prompt(label: &str, default: &str) -> String {
    let suffix = if default.is_empty() {
        String::new()
    } else {
        format!(" [{}]", default)
    };
    print!("{label}{suffix}: ");
    let _ = io::stdout().flush();
    let mut line = String::new();
    if io::stdin().read_line(&mut line).is_err() {
        return default.to_string();
    }
    let line = line.trim();
    if line.is_empty() {
        default.to_string()
    } else {
        line.to_string()
    }
}
