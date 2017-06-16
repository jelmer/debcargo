extern crate cargo;
extern crate chrono;
#[macro_use] extern crate error_chain;
extern crate flate2;
extern crate itertools;
extern crate regex;
extern crate semver;
extern crate semver_parser;
extern crate tar;
extern crate tempdir;
extern crate walkdir;
extern crate subprocess;
extern crate ansi_term;

#[macro_use]
pub mod errors;
pub mod copyright;
pub mod crates;
pub mod debian;
