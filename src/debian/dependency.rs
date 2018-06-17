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

    fn major(&self) -> u64 {
        use self::V::*;
        match *self {
            M(major) | MM(major, _) | MMP(major, _, _) => major,
        }
    }

    fn minor(&self) -> u64 {
        use self::V::*;
        match *self {
            M(_) => 0,
            MM(_, minor) | MMP(_, minor, _) => minor,
        }
    }

    fn inclast(&self) -> V {
        use self::V::*;
        match *self {
            M(major) => M(major + 1),
            MM(major, minor) => MM(major, minor + 1),
            MMP(major, minor, patch) => MMP(major, minor, patch + 1),
        }
    }

    fn incpatch(&self) -> V {
        use self::V::*;
        match *self {
            M(major) => MMP(major, 0, 1),
            MM(major, minor) => MMP(major, minor, 1),
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

fn generate_version_constraints(
    dep: &Dependency,
    p: &semver_parser::range::Predicate,
    op: &semver_parser::range::Op,
    mmp: &V,
) -> Result<Vec<String>> // result is a AND-clause
{
    use debian::dependency::V::*;
    let mut deps : Vec<String> = Vec::new();
    // see https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html
    // for semantics
    match (op, mmp) {
        (&Lt, &M(0)) | (&Lt, &MM(0, 0)) | (&Lt, &MMP(0, 0, 0)) => debcargo_bail!(
            "Unrepresentable dependency version predicate: {} {:?}",
            dep.name(),
            p
        ),
        (&Lt, _) => {
            deps.push(format!("<< {}~~", mmp));
        }
        (&LtEq, _) => {
            deps.push(format!("<< {}~~", mmp.incpatch()));
        }
        (&Gt, _) => {
            deps.push(format!(">= {}~~", mmp.incpatch()));
        }
        (&GtEq, _) => {
            deps.push(format!(">= {}~~", mmp));
        }

        (&Ex, _) => {
            deps.push(format!(">= {}~~", mmp));
            deps.push(format!("<< {}~~", mmp.incpatch()));
        }

        (&Tilde, &M(_)) | (&Tilde, &MM(_, _)) => {
            deps.push(format!(">= {}~~", mmp));
            deps.push(format!("<< {}~~", mmp.inclast()));
        }
        (&Tilde, &MMP(major, minor, _)) => {
            deps.push(format!(">= {}~~", mmp));
            deps.push(format!("<< {}~~", MM(major, minor + 1)));
        }

        (&Compatible, &MMP(0, 0, _)) => {
            deps.push(format!(">= {}~~", mmp));
            deps.push(format!("<< {}~~", mmp.inclast()));
        }
        (&Compatible, &MMP(0, minor, _)) |
        (&Compatible, &MM(0, minor)) => {
            deps.push(format!(">= {}~~", mmp));
            deps.push(format!("<< {}~~", MM(0, minor + 1)));
        }
        (&Compatible, &MMP(major, _, _)) |
        (&Compatible, &MM(major, _)) |
        (&Compatible, &M(major)) => {
            deps.push(format!(">= {}~~", mmp));
            deps.push(format!("<< {}~~", M(major + 1)));
        }

        (&Wildcard(WildcardVersion::Major), _) => {
            ();
        }
        (&Wildcard(WildcardVersion::Minor), _) => {
            deps.push(format!(">= {}~~", mmp.major()));
            deps.push(format!("<< {}~~", mmp.major() + 1));
        }
        (&Wildcard(WildcardVersion::Patch), _) => {
            deps.push(format!(">= {}~~", mmp));
            deps.push(format!("<< {}~~", MM(mmp.major(), mmp.minor() + 1)));
        }
    }

    Ok(deps)
}

/// Translates a Cargo dependency into a Debian package dependency.
pub fn deb_dep(dep: &Dependency) -> Result<Vec<String>> // result is a AND-clause
{
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
        let pkg = format!("librust-{}{}", dep_dashed, suffix);
        for p in &req.predicates {
            // Cargo/semver and Debian handle pre-release versions quite
            // differently, so a versioned Debian dependency cannot properly
            // handle pre-release crates. This might be OK most of the time,
            // coerce it to the non-pre-release version.
            // FIXME: guard this with a config option.
            if !p.pre.is_empty() {
                debcargo_warn!("Stripped off prerelease part of dependency: {} {:?}", dep.name(), p)
            }

            let mmp = V::new(p)?;
            let op = coerce_unacceptable_predicate(dep, &p, &mmp)?;
            let constraints = generate_version_constraints(dep, &p, op, &mmp)?;
            if constraints.is_empty() {
                deps.push(pkg.to_string());
            } else {
                deps.extend(constraints
                    .iter()
                    .map(|c| format!("{} ({})", &pkg, &c)));
            }
        }
    }
    Ok(deps)
}

pub fn deb_deps(cdeps: &Vec<Dependency>) -> Result<Vec<String>> // result is a AND-clause
{
    let mut deps = Vec::new();
    for dep in cdeps {
        deps.extend(deb_dep(dep)?.iter().map(String::to_string));
    }
    Ok(deps)
}
