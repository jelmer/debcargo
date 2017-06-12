use std::io;
use std::string;
use cargo;
use walkdir;
use regex;

error_chain! {
    foreign_links {
        Io(io::Error);
        Cargo(Box<cargo::CargoError>);
        Regex(regex::Error);
        WalkDir(walkdir::Error);
        String(string::FromUtf8Error);
    }
}

#[macro_export]
macro_rules! debcargo_info {
    ($e:expr) => {{
        let mut stdout = StandardStream::stdout(ColorChoice::Auto);
        stdout.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
        writeln!(&mut stdout, $e)?;
    }};

    ($fmt:expr, $($arg:tt)+) => {{
        let mut stdout = StandardStream::stdout(ColorChoice::Auto);
        stdout.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
        writeln!(&mut stdout, "{}", format!($fmt, $($arg)+))?;
    }};
}

#[macro_export]
macro_rules! debcargo_highlight {
    ($e:expr) => {{
        let mut stdout = StandardStream::stdout(ColorChoice::Auto);
        stdout.set_color(ColorSpec::new().set_fg(Some(Color::Red)).set_bold(true).set_intense(true))?;
        writeln!(&mut stdout, $e)?;
    }};

    ($fmt:expr, $($arg:tt)+) => {{
        let mut stdout = StandardStream::stdout(ColorChoice::Auto);
        stdout.set_color(ColorSpec::new().set_fg(Some(Color::Red)).set_bold(true).set_intense(true))?;
        writeln!(&mut stdout, "{}", format!($fmt, $($arg)+))?;
    }};
}

#[macro_export]
macro_rules! debcargo_bail {
    ($e:expr) => {
        return Err(debcargo_bail!($e));
    };

    ($fmt:expr, $($arg:tt)+) => {
        return Err(debcargo_bail!($fmt, $($arg)+));
    }
}
