use cargo::core::Dependency;
use itertools::Itertools;
use semver_parser;
use semver_parser::range::Op::*;
use semver_parser::range::*;

use std::cmp;
use std::fmt;

use crate::config::Config;
use crate::debian;
use crate::errors::*;

#[derive(Eq, Clone)]
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
            (None, Some(_)) => debcargo_bail!("semver had patch without minor"),
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

    fn mmp(&self) -> (u64, u64, u64) {
        use self::V::*;
        match *self {
            M(major) => (major, 0, 0),
            MM(major, minor) => (major, minor, 0),
            MMP(major, minor, patch) => (major, minor, patch),
        }
    }
}

impl cmp::Ord for V {
    fn cmp(&self, other: &V) -> cmp::Ordering {
        self.partial_cmp(other).unwrap()
    }
}

impl cmp::PartialOrd for V {
    fn partial_cmp(&self, other: &V) -> Option<cmp::Ordering> {
        self.mmp().partial_cmp(&other.mmp())
    }
}

impl cmp::PartialEq for V {
    fn eq(&self, other: &V) -> bool {
        self.mmp() == other.mmp()
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

struct VRange {
    ge: Option<V>,
    lt: Option<V>,
}

impl VRange {
    fn new() -> Self {
        VRange { ge: None, lt: None }
    }

    fn constrain_ge(&mut self, ge: V) -> &Self {
        match self.ge {
            Some(ref ge_) if &ge < ge_ => (),
            _ => self.ge = Some(ge),
        };
        self
    }

    fn constrain_lt(&mut self, lt: V) -> &Self {
        match self.lt {
            Some(ref lt_) if &lt >= lt_ => (),
            _ => self.lt = Some(lt),
        };
        self
    }

    fn to_deb_or_clause(&self, base: &str, suffix: &str) -> Result<String> {
        use debian::dependency::V::*;
        match (&self.ge, &self.lt) {
            (None, None) => Ok(format!("{}{}", base, suffix)),
            (Some(ge), None) => Ok(format!("{}{} (>= {}-~~)", base, suffix, ge)),
            (None, Some(lt)) => Ok(format!("{}{} (<< {}-~~)", base, suffix, lt)),
            (Some(ge), Some(lt)) => {
                if ge >= lt {
                    debcargo_bail!("bad version range: >= {}, << {}", ge, lt);
                }
                let mut ranges = vec![];
                let (lt_maj, lt_min, lt_pat) = lt.mmp();
                let (ge_maj, ge_min, ge_pat) = ge.mmp();
                if ge_maj < lt_maj {
                    ranges.push((M(ge_maj), Some((true, ge))));
                    ranges.extend((ge_maj + 1..lt_maj).map(|maj| (M(maj), None)));
                    ranges.push((M(lt_maj), Some((false, lt))));
                } else {
                    assert!(ge_maj == lt_maj);
                    if ge_min < lt_min {
                        ranges.push((MM(ge_maj, ge_min), Some((true, ge))));
                        ranges.extend((ge_min + 1..lt_min).map(|min| (MM(ge_maj, min), None)));
                        ranges.push((MM(lt_maj, lt_min), Some((false, lt))));
                    } else {
                        assert!(ge_min == lt_min);
                        ranges.push((MMP(ge_maj, ge_min, ge_pat), Some((true, ge))));
                        ranges.extend(
                            (ge_pat + 1..lt_pat).map(|pat| (MMP(ge_maj, ge_min, pat), None)),
                        );
                        ranges.push((MMP(lt_maj, lt_min, lt_pat), Some((false, lt))));
                    }
                };
                // reverse the order so higher versions go first
                // this helps sbuild find build-deps, it does not resolve alternatives by default
                Ok(ranges
                    .iter()
                    .rev()
                    .filter_map(|(ver, cons)| match cons {
                        None => Some(format!("{}-{}{}", base, ver, suffix)),
                        Some((true, c)) => {
                            if c == &ver {
                                // A-x >= x is redundant, drop the >=
                                Some(format!("{}-{}{}", base, ver, suffix))
                            } else {
                                Some(format!("{}-{}{} (>= {}-~~)", base, ver, suffix, c))
                            }
                        }
                        Some((false, c)) => {
                            if c == &ver {
                                // A-x << x is unsatisfiable, drop it
                                None
                            } else {
                                Some(format!("{}-{}{} (<< {}-~~)", base, ver, suffix, c))
                            }
                        }
                    })
                    .join(" | "))
            }
        }
    }
}

fn coerce_unacceptable_predicate<'a>(
    dep: &Dependency,
    p: &'a semver_parser::range::Predicate,
    allow_prerelease_deps: bool,
) -> Result<&'a semver_parser::range::Op> {
    let mmp = &V::new(p)?;

    // Cargo/semver and Debian handle pre-release versions quite
    // differently, so a versioned Debian dependency cannot properly
    // handle pre-release crates. This might be OK most of the time,
    // coerce it to the non-pre-release version.
    if !p.pre.is_empty() {
        if allow_prerelease_deps {
            debcargo_warn!(
                "Coercing removal of prerelease part of dependency: {} {:?}",
                dep.package_name(),
                p
            )
        } else {
            debcargo_bail!(
                "Cannot represent prerelease part of dependency: {} {:?}",
                dep.package_name(),
                p
            )
        }
    }

    use debian::dependency::V::M;
    match (&p.op, mmp) {
        (&Gt, &M(0)) => Ok(&p.op),
        (&GtEq, &M(0)) => {
            debcargo_warn!(
                "Coercing unrepresentable dependency version predicate 'GtEq 0' to 'Gt 0': {} {:?}",
                dep.package_name(),
                p
            );
            Ok(&Gt)
        }
        // TODO: This will prevent us from handling wildcard dependencies with
        // 0.0.0* so for now commenting this out.
        // (_, &M(0)) => debcargo_bail!(
        //     "Unrepresentable dependency version predicate: {} {:?}",
        //     dep.package_name(),
        //     p
        // ),
        (_, _) => Ok(&p.op),
    }
}

fn generate_version_constraints(
    vr: &mut VRange,
    dep: &Dependency,
    p: &semver_parser::range::Predicate,
    op: &semver_parser::range::Op,
) -> Result<()> {
    let mmp = V::new(p)?;
    use debian::dependency::V::*;
    // see https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html
    // for semantics
    match (op, &mmp.clone()) {
        (&Lt, &M(0)) | (&Lt, &MM(0, 0)) | (&Lt, &MMP(0, 0, 0)) => debcargo_bail!(
            "Unrepresentable dependency version predicate: {} {:?}",
            dep.package_name(),
            p
        ),
        (&Lt, _) => {
            vr.constrain_lt(mmp);
        }
        (&LtEq, _) => {
            vr.constrain_lt(mmp.incpatch());
        }
        (&Gt, _) => {
            vr.constrain_ge(mmp.incpatch());
        }
        (&GtEq, _) => {
            vr.constrain_ge(mmp);
        }

        (&Ex, _) => {
            vr.constrain_lt(mmp.incpatch());
            vr.constrain_ge(mmp);
        }

        (&Tilde, &M(_)) | (&Tilde, &MM(_, _)) => {
            vr.constrain_lt(mmp.inclast());
            vr.constrain_ge(mmp);
        }
        (&Tilde, &MMP(major, minor, _)) => {
            vr.constrain_lt(MM(major, minor + 1));
            vr.constrain_ge(mmp);
        }

        (&Compatible, &MMP(0, 0, _)) => {
            vr.constrain_lt(mmp.inclast());
            vr.constrain_ge(mmp);
        }
        (&Compatible, &MMP(0, minor, _)) | (&Compatible, &MM(0, minor)) => {
            vr.constrain_lt(MM(0, minor + 1));
            vr.constrain_ge(mmp);
        }
        (&Compatible, &MMP(major, _, _))
        | (&Compatible, &MM(major, _))
        | (&Compatible, &M(major)) => {
            vr.constrain_lt(M(major + 1));
            vr.constrain_ge(mmp);
        }

        (&Wildcard(WildcardVersion::Minor), _) => {
            vr.constrain_lt(M(mmp.major() + 1));
            vr.constrain_ge(M(mmp.major()));
        }
        (&Wildcard(WildcardVersion::Patch), _) => {
            vr.constrain_lt(MM(mmp.major(), mmp.minor() + 1));
            vr.constrain_ge(mmp);
        }
    }

    Ok(())
}

/// Translates a Cargo dependency into a Debian package dependency.
pub fn deb_dep(config: &Config, dep: &Dependency) -> Result<Vec<String>> // result is a AND-clause
{
    let dep_dashed = dep.package_name().replace('_', "-").to_lowercase();
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
        let base = format!("librust-{}", dep_dashed);
        let mut vr = VRange::new();
        for p in &req.predicates {
            let op = coerce_unacceptable_predicate(dep, &p, config.allow_prerelease_deps)?;
            generate_version_constraints(&mut vr, dep, &p, op)?;
        }
        deps.push(vr.to_deb_or_clause(&base, &suffix)?);
    }
    Ok(deps)
}

pub fn deb_deps(config: &Config, cdeps: &[Dependency]) -> Result<Vec<String>> // result is a AND-clause
{
    let mut deps = Vec::new();
    for dep in cdeps {
        deps.extend(deb_dep(config, dep)?.iter().map(String::to_string));
    }
    deps.sort();
    deps.dedup();
    Ok(deps)
}

pub fn deb_dep_add_nocheck(x: &str) -> String {
    x.to_string()
        .split('|')
        .map(|x| x.trim_end().to_string() + " <!nocheck> ")
        .join("|")
        .trim_end()
        .to_string()
}
