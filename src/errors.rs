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
