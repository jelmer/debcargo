use std::io;
use std::string;
use std::num;
use cargo;
use walkdir;
use regex;
use subprocess;

error_chain! {
    foreign_links {
        Io(io::Error);
        Cargo(Box<cargo::CargoError>);
        Regex(regex::Error);
        WalkDir(walkdir::Error);
        String(string::FromUtf8Error);
        Subprocess(subprocess::PopenError);
        ParseInt(num::ParseIntError);
    }
}

#[macro_export]
macro_rules! debcargo_info {
    ($e:expr) => {
        println!("{}",Blue.paint($e));
    };
}

#[macro_export]
macro_rules! debcargo_warn {
    ($e:expr) => {
        println!("{}", RGB(255,165,0).bold().paint($e))
    };

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
