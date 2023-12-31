use anyhow;

pub type Result<T> = ::std::result::Result<T, anyhow::Error>;
pub use anyhow::format_err;

#[macro_export]
macro_rules! debcargo_info {
    ($e:expr) => {
        {
            use ansi_term::Colour::Green;
            eprintln!("{}", Green.paint($e));
        }
    };

    ($fmt:expr, $( $arg:tt)+) => {
        {
            use ansi_term::Colour::Green;
            let print_string = format!($fmt, $($arg)+);
            eprintln!("{}", Green.paint(print_string));
        }
    };
}

#[macro_export]
macro_rules! debcargo_warn {
    ($e:expr) => {
        {
            use ansi_term::Colour::RGB;
            eprintln!("{}", RGB(255,165,0).bold().paint($e));
        }
    };

    ($fmt:expr, $( $arg:tt)+) => {
        {
            use ansi_term::Colour::RGB;
            let print_string = RGB(255,165,0).bold().paint(format!($fmt, $($arg)+));
            eprintln!("{}", print_string);
        }
    };

}

#[macro_export]
macro_rules! debcargo_bail {
    ($e:expr) => {{
        return Err(::anyhow::format_err!("{}", $e));
    }};

    ($fmt:expr, $( $arg:tt)+) => {
        {
            let error_string = format!($fmt, $($arg)+);
            return Err(::anyhow::format_err!("{}", error_string));
        }
    };
}
