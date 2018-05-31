use semver_parser;
use semver_parser::range::*;
use semver_parser::range::Op::*;
use cargo::core::Dependency;

use std::fmt;

use errors::*;
use crates::CratesIo;

#[derive(PartialEq)]
enum V {
    M(u64),
    MM(u64, u64),
    MMP(u64, u64, u64),
}

impl V {
    fn new(p: &Predicate) -> Result<Self> {
        use self::V::*;
        let mmp = match (p.minor, p.patch) {
            (None, None) => M(p.major),
            (Some(minor), None) => MM(p.major, minor),
            (Some(minor), Some(patch)) => MMP(p.major, minor, patch),
            (None, Some(_)) => panic!("semver had patch without minor"),
        };
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

fn coerce_unacceptable_predicate<'a>(
    dep: &Dependency,
    p: &'a semver_parser::range::Predicate,
    mmp: &V,
) -> Result<&'a semver_parser::range::Op> {
    use debian::dependency::V::M;
    match (&p.op, mmp) {
        (&Gt, &M(0)) => Ok(&p.op),
        (&GtEq, &M(0)) => {
            debcargo_warn!(
                "Coercing unrepresentable dependency version predicate 'GtEq 0' to 'Gt 0': {} {:?}",
                dep.name(),
                p
            );
            Ok(&Gt)
        }
        // TODO: This will prevent us from handling wildcard dependencies with
        // 0.0.0* so for now commenting this out.
        // (_, &M(0)) => debcargo_bail!(
        //     "Unrepresentable dependency version predicate: {} {:?}",
        //     dep.name(),
        //     p
        // ),
        (_, _) => Ok(&p.op),
    }
}

fn generate_package_name<F>(
    dep: &Dependency,
    pkg: &F,
    p: &semver_parser::range::Predicate,
    op: &semver_parser::range::Op,
    mmp: &V,
) -> Result<Vec<String>>
where
    F: Fn(&V) -> String,
{
    use debian::dependency::V::*;
    let mut deps = Vec::new();
    match (op, mmp) {
        (&Ex, &M(..)) => deps.push(pkg(&mmp)),
        (&Ex, &MM(..)) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
        (&Ex, &MMP(..)) => {
            deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
            deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast()));
        }
        // We can't represent every major version that satisfies an
        // inequality, because each major version has a different Debian
        // package name, so we (for now) use the first few major versions
        // that satisfies the inequality. This may result in a stricter
        // dependency, but will never result in a looser one. We could
        // represent some dependency ranges (such as >= x and < y)
        // better with a disjunction on multiple package names, but that
        // would break when depending on multiple features.
        (&Gt, &M(major)) | (&Gt, &MM(major, _)) | (&Gt, &MMP(major, _, _)) => {
            deps.push(format!("{} (>> {})| {} | {} | {}",
                pkg(&mmp), mmp,
                pkg(&M(major + 1)),
                pkg(&M(major + 2)),
                pkg(&M(major + 3))))
        },
        (&GtEq, &M(major)) | (&GtEq, &MM(major, _)) | (&GtEq, &MMP(major, _, _))=> {
            deps.push(format!("{} (>= {}) | {} | {} | {}",
                pkg(&mmp), mmp,
                pkg(&M(major + 1)),
                pkg(&M(major + 2)),
                pkg(&M(major + 3))))
        }
        (&Lt, &M(major)) => deps.push(pkg(&M(major - 1))),
        (&Lt, &MM(0, 0)) => debcargo_bail!(
            "Unrepresentable dependency version predicate: {} {:?}",
            dep.name(),
            p
        ),
        (&Lt, &MM(0, minor)) | (&Lt, &MMP(0, minor, 0)) => deps.push(pkg(&MM(0, minor - 1))),
        (&Lt, _) => deps.push(format!("{} (<< {})", pkg(&mmp), mmp)),
        (&LtEq, &M(_)) | (&LtEq, &MM(0, _)) => deps.push(pkg(&mmp)),
        (&LtEq, _) => deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast())),
        (&Tilde, &M(_)) | (&Tilde, &MM(0, _)) | (&Tilde, &MMP(0, _, 0)) => deps.push(pkg(&mmp)),
        (&Tilde, &MM(..)) | (&Tilde, &MMP(0, _, _)) => {
            deps.push(format!("{} (>= {})", pkg(&mmp), mmp))
        }
        (&Tilde, &MMP(major, minor, _)) => {
            deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
            deps.push(format!("{} (<< {})", pkg(&mmp), MM(major, minor + 1)));
        }
        (&Compatible, &MMP(0, 0, _)) => {
            deps.push(format!("{} (>= {})", pkg(&mmp), mmp));
            deps.push(format!("{} (<< {})", pkg(&mmp), mmp.inclast()));
        }
        (&Compatible, &M(_))
        | (&Compatible, &MM(0, _))
        | (&Compatible, &MM(_, 0))
        | (&Compatible, &MMP(0, _, 0)) => deps.push(pkg(&mmp)),
        (&Compatible, &MM(..)) | (&Compatible, &MMP(..)) => {
            deps.push(format!("{} (>= {})", pkg(&mmp), mmp))
        }
        (&Wildcard(WildcardVersion::Major), _) => {
            // We take all possible version from the crates io which will be
            // returned to us as sorted dependency. We take first few lets say 5
            // and use it as alternative dependencies.
            let crates_io = CratesIo::new()?;
            let mut candidates = crates_io.fetch_as_dependency(dep)?;

            // TODO: What happens if there are less than 5 elements?.
            for d in candidates.iter().take(5) {
                let req = semver_parser::range::parse(&d.version_req().to_string()).unwrap();
                let mmp = V::new(&req.predicates[0])?;
                deps.push(pkg(&mmp));
            }

            // Possibly multiple minor version changes leads duplicate
            // dependencies but luckily consecutive so lets drop them
            deps.dedup();
        }
        (&Wildcard(WildcardVersion::Minor), _) => deps.push(pkg(&mmp)),
        (&Wildcard(WildcardVersion::Patch), _) => deps.push(format!("{} (>= {})", pkg(&mmp), mmp)),
    }

    Ok(deps)
}

/// Translates a Cargo dependency into a Debian package dependency.
pub fn deb_dep(dep: &Dependency) -> Result<Vec<String>> {
    use self::V::*;
    let dep_dashed = dep.name().replace('_', "-");
    let mut suffixes = Vec::new();
    if dep.uses_default_features() {
        suffixes.push("+default-dev".to_string());
    }
    for feature in dep.features() {
        suffixes.push(format!("+{}-dev", feature.replace('_', "-").to_lowercase()));
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
                MM(major, minor) | MMP(major, minor, _) => (major, minor),
            };
            if major == 0 {
                format!("librust-{}-{}.{}{}", dep_dashed, major, minor, suffix)
            } else {
                format!("librust-{}-{}{}", dep_dashed, major, suffix)
            }
        };

        if req.predicates.len() == 1 {
            let p = &req.predicates[0];
            let mmp = V::new(p)?;
            let op = coerce_unacceptable_predicate(dep, &p, &mmp)?;
            deps.extend(generate_package_name(dep, &pkg, &p, op, &mmp)?);
        } else {
            let mut mdeps = Vec::new();
            for p in &req.predicates {
                // Cargo/semver and Debian handle pre-release versions quite
                // differently, so a versioned Debian dependency cannot properly
                // handle pre-release crates. Don't package pre-release crates or
                // crates that depend on pre-release crates.
                if !p.pre.is_empty() {
                    debcargo_bail!("Dependency on prerelease version: {} {:?}", dep.name(), p)
                }

                let mmp = V::new(p)?;
                let op = coerce_unacceptable_predicate(dep, &p, &mmp)?;
                mdeps.extend(generate_package_name(dep, &pkg, &p, op, &mmp)?)
            }

            deps.push(mdeps.join(" | "));
        }
    }
    Ok(deps)
}

pub fn deb_deps(cdeps: &Vec<Dependency>) -> Result<Vec<String>> {
    let mut deps = Vec::new();
    for dep in cdeps {
        deps.extend(deb_dep(dep)?);
    }

    deps.sort();
    deps.dedup();
    Ok(deps)
}
