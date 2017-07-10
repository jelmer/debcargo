use std::io;
use std::string;
use std::num;
use cargo;
use walkdir;
use regex;
use toml;
use git2;

error_chain! {
    foreign_links {
        Io(io::Error);
        Cargo(Box<cargo::CargoError>);
        Regex(regex::Error);
        WalkDir(walkdir::Error);
        String(string::FromUtf8Error);
        ParseInt(num::ParseIntError);
        TomlError(toml::de::Error);
        GitError(git2::Error);
    }
}

#[macro_export]
macro_rules! debcargo_info {
    ($e:expr) => {
        {
            use ansi_term::Colour::Green;
            println!("{}", Green.paint($e));
        }
    };

    ($fmt:expr, $( $arg:tt)+) => {
        {
            use ansi_term::Colour::Green;
            let print_string = format!($fmt, $($arg)+);
            println!("{}", Green.paint(print_string));
        }
    };
}

#[macro_export]
macro_rules! debcargo_warn {
    ($e:expr) => {
        {
            use ansi_term::Colour::RGB;
            println!("{}", RGB(255,165,0).bold().paint($e));
        }
    };

    ($fmt:expr, $( $arg:tt)+) => {
        {
            use ansi_term::Colour::RGB;
            let print_string = RGB(255,165,0).bold.paint(format!($fmt, $($arg)+));
            println!("{}", print_string);
        }
    };

}

#[macro_export]
macro_rules! debcargo_bail {
    ($e:expr) => {{
        use ansi_term::Colour::Red;
        let error_string = Red.bold().paint($e).to_string();
        return Err(error_string.into());
    }};

    ($fmt:expr, $( $arg:tt)+) => {
        {
            use ansi_term::Colour::Red;
            let error_string = format!($fmt, $($arg)+);
            return Err(Red.bold().paint(error_string).to_string().into());
        }
    };
}
