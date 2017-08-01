use semver_parser;
use semver_parser::range::*;
use semver_parser::range::Op::*;
use cargo::core::Dependency;

use std::fmt;

use errors::*;

#[derive(PartialEq)]
enum V {
    M(u64),
    MM(u64, u64),
    MMP(u64, u64, u64),
}

impl V {
    fn new(p: &Predicate, dep: &str) -> Result<Self> {
        use self::V::*;
        let mmp = match (p.minor, p.patch) {
            (None, None) => M(p.major),
            (Some(minor), None) => MM(p.major, minor),
            (Some(minor), Some(patch)) => MMP(p.major, minor, patch),
            (None, Some(_)) => panic!("semver had patch without minor"),
        };
        if mmp == M(0) && p.op != Gt {
            debcargo_bail!("Unrepresentable dependency version predicate: {} {:?}",
                           dep,
                           p);
        }

        Ok(mmp)
    }

    fn inclast(&self) -> V {
        use self::V::*;
        match *self {
            M(major) => M(major + 1),
            MM(major, minor) => MM(major, minor + 1),
            MMP(major, minor, patch) => MMP(major, minor, patch + 1),
        }
    }
}

impl fmt::Display for V {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use self::V::*;
        match *self {
            M(major) => write!(f, "{}", major),
            MM(major, minor) => write!(f, "{}.{}", major, minor),
            MMP(major, minor, patch) => write!(f, "{}.{}.{}", major, minor, patch),
        }
    }
}

/// Translates a Cargo dependency into a Debian package dependency.
pub fn deb_dep(dep: &Dependency) -> Result<String> {
    use self::V::*;
    let dep_dashed = dep.name().replace('_', "-");
    let mut suffixes = Vec::new();
    if dep.uses_default_features() {
        suffixes.push("+default-dev".to_string());
    }
    for feature in dep.features() {
        suffixes.push(format!("+{}-dev", feature.replace('_', "-")));
    }
    if suffixes.is_empty() {
        suffixes.push("-dev".to_string());
    }
    let req = semver_parser::range::parse(&dep.version_req().to_string()).unwrap();
    let mut deps = Vec::new();
    for suffix in suffixes {
        let pkg = |v: &V| {
            let (major, minor) = match *v {
                M(major) => (major, 0),
                MM(major, minor) |
                MMP(major, minor, _) => (major, minor),
            };
            if major == 0 {
                format!("librust-{}-{}.{}{}", dep_dashed, major, minor, suffix)
            } else {
                format!("librust-{}-{}{}", dep_dashed, major, suffix)
            }
        };

        for p in &req.predicates {
            // Cargo/semver and Debian handle pre-release versions quite
            // differently, so a versioned Debian dependency cannot properly
            // handle pre-release crates. Don't package pre-release crates or
            // crates that depend on pre-release crates.
            if !p.pre.is_empty() {
                debcargo_bail!("Dependency on prerelease version: {} {:?}", dep.name(), p);
            }

            let mmp = V::new(p, dep.name())?;

            match (&p.op, &mmp) {
                (&Ex, &M(..)) => deps.push(pkg(&mmp)),
                (&Ex, &MM(..)) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Ex, &MMP(..)) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
                    deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast()));
                }
                // We can't represent every major version that satisfies an
                // inequality, because each major version has a different
                // package name, so we only allow the first major version that
                // satisfies the inequality. This may result in a stricter
                // dependency, but will never result in a looser one. We could
                // represent some dependency ranges (such as >= x and < y)
                // better with a disjunction on multiple package names, but that
                // would break when depending on multiple features.
                (&Gt, &M(_)) | (&Gt, &MM(0, _)) => deps.push(pkg(&mmp.inclast())),
                (&Gt, _) => deps.push(format!("{} (>> {})", pkg(&mmp), mmp)),
                (&GtEq, &M(_)) |
                (&GtEq, &MM(0, _)) => deps.push(pkg(&mmp)),
                (&GtEq, _) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Lt, &M(major)) => deps.push(pkg(&M(major - 1))),
                (&Lt, &MM(0, 0)) => {
                    debcargo_bail!("Unrepresentable dependency version predicate: {} {:?}",
                                   dep.name(),
                                   p)
                }
                (&Lt, &MM(0, minor)) => deps.push(pkg(&MM(0, minor - 1))),
                (&Lt, _) => deps.push(format!("{} (<< {})", pkg(&mmp), mmp)),
                (&LtEq, &M(_)) |
                (&LtEq, &MM(0, _)) => deps.push(pkg(&mmp)),
                (&LtEq, _) => deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast())),
                (&Tilde, &M(_)) |
                (&Tilde, &MM(0, _)) |
                (&Tilde, &MMP(0, _, 0)) => deps.push(pkg(&mmp)),
                (&Tilde, &MM(..)) |
                (&Tilde, &MMP(0, _, _)) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Tilde, &MMP(major, minor, _)) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
                    deps.push(format!("{} (<< {})", pkg(&mmp), MM(major, minor + 1)));
                }
                (&Compatible, &MMP(0, 0, _)) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
                    deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast()));
                }
                (&Compatible, &M(_)) |
                (&Compatible, &MM(0, _)) |
                (&Compatible, &MM(_, 0)) |
                (&Compatible, &MMP(0, _, 0)) => deps.push(pkg(&mmp)),
                (&Compatible, &MM(..)) |
                (&Compatible, &MMP(..)) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
                (&Wildcard(WildcardVersion::Major), _) => {
                    debcargo_bail!("Unrepresentable dependency wildcard: {} = \"{:?}\"",
                                   dep.name(),
                                   p)
                }
                (&Wildcard(WildcardVersion::Minor), _) => deps.push(pkg(&mmp)),
                (&Wildcard(WildcardVersion::Patch), _) => {
                    deps.push(format!("{} (>= {})", pkg(&mmp), mmp))
                }
            }
        }
    }
    Ok(deps.join(", "))
}