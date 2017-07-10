#[macro_use]
extern crate error_chain;
#[macro_use]
extern crate serde_derive;
extern crate cargo;
extern crate chrono;
extern crate flate2;
extern crate itertools;
extern crate regex;
extern crate semver;
extern crate semver_parser;
extern crate tar;
extern crate tempdir;
extern crate walkdir;
extern crate ansi_term;
extern crate toml;
extern crate git2;

#[macro_use]
pub mod errors;
pub mod copyright;
pub mod crates;
pub mod debian;
pub mod overrides;
