use failure;

pub type Result<T> = ::std::result::Result<T, failure::Error>;
pub use failure::{format_err, ResultExt};

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
            let print_string = RGB(255,165,0).bold().paint(format!($fmt, $($arg)+));
            println!("{}", print_string);
        }
    };

}

#[macro_export]
macro_rules! debcargo_bail {
    ($e:expr) => {{
        return Err(format_err!("{}", $e));
    }};

    ($fmt:expr, $( $arg:tt)+) => {
        {
            let error_string = format!($fmt, $($arg)+);
            return Err(format_err!("{}", error_string));
        }
    };
}
