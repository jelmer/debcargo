use chrono;
use textwrap::fill;

use std::fmt;
use std::iter::FromIterator;


pub struct Changelog<'a> {
    source: &'a str,
    version: &'a str,
    distribution: &'a str,
    urgency: &'a str,
    maintainer: &'a str,
    entries: Vec<String>,
}

impl<'a> fmt::Display for Changelog<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "{} ({}) {}; urgency={}\n",
            self.source,
            self.version,
            self.distribution,
            self.urgency
        )?;

        for entry in self.entries.iter() {
            writeln!(f, "  * {}", fill(entry, 79))?;
        }

        writeln!(
            f,
            "\n -- {}  {}\n",
            self.maintainer,
            chrono::Local::now().to_rfc2822()
        )
    }
}

impl<'a> Changelog<'a> {
    pub fn new(
        src: &'a str,
        version: &'a str,
        distribution: &'a str,
        urgency: &'a str,
        maintainer: &'a str,
        entries: &[String],
    ) -> Self {
        let changelog = entries.iter().map(|x| x.to_string());
        Changelog {
            source: src,
            version: version,
            distribution: distribution,
            urgency: urgency,
            maintainer: maintainer,
            entries: Vec::from_iter(changelog),
        }
    }
}
