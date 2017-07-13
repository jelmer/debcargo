use semver::Version;
use std::env::{self, VarError};
use std::fmt::Write;

use errors::*;

pub use self::source::Source;
pub use self::package::Package;

pub mod source;
pub mod package;

/// Translates a semver into a Debian version. Omits the build metadata, and uses a ~ before the
/// prerelease version so it compares earlier than the subsequent release.
pub fn deb_version(v: &Version) -> String {
    let mut s = format!("{}.{}.{}", v.major, v.minor, v.patch);
    for (n, id) in v.pre.iter().enumerate() {
        write!(s, "{}{}", if n == 0 { '~' } else { '.' }, id).unwrap();
    }
    s
}

fn deb_name(name: &str) -> String {
    format!("librust-{}-dev", name.replace('_', "-"))
}

pub fn deb_feature_name(name: &str, feature: &str) -> String {
    format!("librust-{}+{}-dev",
            name.replace('_', "-"),
            feature.replace('_', "-"))
}

/// Retrieve one of a series of environment variables, and provide a friendly error message for
/// non-UTF-8 values.
fn get_envs(keys: &[&str]) -> Result<Option<String>> {
    for key in keys {
        match env::var(key) {
            Ok(val) => {
                return Ok(Some(val));
            }
            Err(e @ VarError::NotUnicode(_)) => {
                return Err(e)
                    .chain_err(|| format!("Environment variable ${} not valid UTF-8", key));
            }
            Err(VarError::NotPresent) => {}
        }
    }
    Ok(None)
}

/// Determine a name and email address from environment variables.
pub fn get_deb_author() -> Result<String> {
    let name = try!(try!(get_envs(&["DEBFULLNAME", "NAME"]))
                        .ok_or("Unable to determine your name; please set $DEBFULLNAME or $NAME"));
    let email = try!(try!(get_envs(&["DEBEMAIL", "EMAIL"]))
                         .ok_or("Unable to determine your email; please set $DEBEMAIL or $EMAIL"));
    Ok(format!("{} <{}>", name, email))
}
