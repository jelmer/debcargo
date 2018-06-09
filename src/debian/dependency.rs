use semver::Version;
use semver_parser;
use semver_parser::range::*;
use semver_parser::range::Op::*;
use cargo::core::Dependency;

use std::fmt;
use std::cmp::Ord;

use errors::*;
use crates::CratesIo;
use itertools::Itertools;

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

    fn new_v(v: &Version) -> Self {
        use self::V::*;
        MMP(v.major, v.minor, v.patch)
    }

    fn major(&self) -> u64 {
        use self::V::*;
        match *self {
            M(major) | MM(major, _) | MMP(major, _, _) => major,
        }
    }

    fn minor_0(&self) -> Option<u64> {
        use self::V::*;
        match *self {
            MM(0, minor) | MMP(_, minor, _) => Some(minor),
            _ => None,
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
) -> Result<Vec<(String, Option<String>)>> // result is a OR-clause
where
    F: Fn(&V) -> String,
{
    use debian::dependency::V::*;
    let mut deps : Vec<(String, Option<String>)> = Vec::new();
    // see https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html
    // for semantics
    match (op, mmp) {
        // We can't represent every version that satisfies a Gt/GtEq
        // inequality, because each semver version has a different Debian
        // package name, so we (for now) use the first few major versions
        // that satisfies the inequality, or first few minor versions if the
        // major version is zero. This may result in a stricter dependency,
        // but will never result in a looser one.
        //
        // TODO, clarify the following comment made by Josh: We could
        // represent some dependency ranges (such as >= x and < y)
        // better with a disjunction on multiple package names, but that
        // would break when depending on multiple features.
        (&Gt, _) | (&GtEq, _) => {
            // TODO: find the currently-available one on crates.io and put it
            // at the top so `sbuild` works without --resolve-alternatives
            let ops = if *op == Gt { ">>" } else { ">=" };
            let major = mmp.major();
            deps.push((pkg(&mmp), Some(format!("{} {}", ops, mmp))));
            if major == 0 {
                let minor = mmp.minor_0().unwrap();
                deps.extend((minor+1..minor+5).map(|v| (pkg(&MM(0, v)), None)));
            }
            deps.extend((major+1..major+5).map(|v| (pkg(&M(v)), None)));
        },
        (&Lt, &M(0)) | (&Lt, &MM(0, 0)) | (&Lt, &MMP(0, 0, 0)) => debcargo_bail!(
            "Unrepresentable dependency version predicate: {} {:?}",
            dep.name(),
            p
        ),
        (&Lt, _) | (&LtEq, _) => {
            let ops = if *op == Lt { "<<" } else { "<=" };
            let major = mmp.major();
            // note that the "{} (<< {})" case is unsatisfiable e.g. when minor = 0, patch = 0
            // but this is fine because of the other alternatives
            deps.push((pkg(&mmp), Some(format!("{} {}", ops, mmp))));
            if major >= 1 {
                // if major > 0 they probably don't care about 0.x versions
                deps.extend((1..major).rev().map(|v| (pkg(&M(v)), None)))
            } else {
                let minor = mmp.minor_0().unwrap();
                deps.extend((0..minor).rev().map(|v| (pkg(&MM(0, v)), None)))
            }
        },

        (&Ex, &M(..)) => {
            deps.push((pkg(&mmp), None));
        }
        (&Ex, &MM(..)) => {
            deps.push((pkg(&mmp), Some(format!(">= {}", mmp))));
        }
        (&Ex, &MMP(..)) => {
            deps.push((pkg(&mmp), Some(format!(">= {}", mmp))));
            deps.push((pkg(&mmp), Some(format!("<< {}", mmp.inclast()))));
        }

        (&Tilde, &M(_)) | (&Tilde, &MM(0, _)) | (&Tilde, &MMP(0, _, 0)) => {
            deps.push((pkg(&mmp), None));
        }
        (&Tilde, &MM(..)) | (&Tilde, &MMP(0, _, _)) => {
            deps.push((pkg(&mmp), Some(format!(">= {}", mmp))));
        }
        (&Tilde, &MMP(major, minor, _)) => {
            deps.push((pkg(&mmp), Some(format!(">= {}", mmp))));
            deps.push((pkg(&mmp), Some(format!("<< {}", MM(major, minor + 1)))));
        }

        (&Compatible, &MMP(0, 0, _)) => {
            deps.push((pkg(&mmp), Some(format!(">= {}", mmp))));
            deps.push((pkg(&mmp), Some(format!("<< {}", mmp.inclast()))));
        }
        (&Compatible, &M(_)) |
        (&Compatible, &MM(0, _)) |
        (&Compatible, &MM(_, 0)) |
        (&Compatible, &MMP(0, _, 0)) => {
            deps.push((pkg(&mmp), None));
        }
        (&Compatible, &MM(..)) |
        (&Compatible, &MMP(..))  => {
            deps.push((pkg(&mmp), Some(format!(">= {}", mmp))));
        }

        (&Wildcard(WildcardVersion::Major), _) => {
            // We take all possible version from the crates io which will be
            // returned to us as sorted dependency. We take all and use it as
            // alternative dependencies, preferring the latest.
            let crates_io = CratesIo::new()?;
            let mut candidates = crates_io.fetch_candidates(dep)?;
            let mut vdeps = Vec::new();

            for s in candidates.iter() {
                vdeps.push(s.version());
            }
            vdeps.sort_by(|a, b| b.cmp(a));
            let mut vdeps_pkg = vdeps
                .iter()
                .map(|p| (pkg(&V::new_v(&p)), None))
                .collect::<Vec<_>>();
            vdeps_pkg.dedup();
            deps.extend(vdeps_pkg.into_iter());
        }
        (&Wildcard(WildcardVersion::Minor), _) => {
            deps.push((pkg(&mmp), None));
        }
        (&Wildcard(WildcardVersion::Patch), _) => {
            deps.push((pkg(&mmp), Some(format!(">= {}", mmp))));
        }
    }

    Ok(deps)
}

/// Translates a Cargo dependency into a Debian package dependency.
pub fn deb_dep(dep: &Dependency) -> Result<Vec<String>> // result is a AND-clause
{
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

        let mut mdeps = Vec::new();
        let mut all_candidates: Vec<String> = Vec::new();
        for p in &req.predicates {
            // Cargo/semver and Debian handle pre-release versions quite
            // differently, so a versioned Debian dependency cannot properly
            // handle pre-release crates. This might be OK most of the time,
            // coerce it to the non-pre-release version.
            if !p.pre.is_empty() {
                debcargo_warn!("Stripped off prerelease part of dependency: {} {:?}", dep.name(), p)
            }

            let mmp = V::new(p)?;
            let op = coerce_unacceptable_predicate(dep, &p, &mmp)?;
            let ors = generate_package_name(dep, &pkg, &p, op, &mmp)?;
            all_candidates.extend(ors.iter().map(|(x, _)| x.to_string()));
            mdeps.push(ors);
        }
        all_candidates.sort();
        all_candidates.dedup();
        // prune out unsatisfiable combinations, helps to solve #7
        let dep = all_candidates.iter().filter_map(|p| {
            // filter out all candidates that don't appear in all OR clauses
            // also detect when a candidate has multiple constraints defined
            // this is usually a fuck-up by the crate maintainer
            mdeps.iter().fold(Some(None), |acc: Option<Option<String>>, ors| {
                if let Some(cons) = acc {
                    let mut it = ors.iter().filter(|&(p_, _)| p_ == p);
                    if let Some((_, cons_)) = it.next() {
                        assert!(it.next().is_none());
                        match (cons, cons_) {
                            (Some(ref x), Some(ref y)) => {
                                debcargo_warn!("Crate somehow had two constraints for dependency {}: {}, {}; we'll use the first one, but this might be wrong", p, x, y);
                                Some(Some(x.to_string()))
                            },
                            (None, Some(x)) => Some(Some(x.to_string())),
                            (r, None) => Some(r),
                        }
                    } else {
                        // candidate not in this OR-clause, can't be satisfied
                        None
                    }
                } else {
                    // propagate None from previous candidate
                    None
                }
            }).map(|constraint| match constraint {
                Some(c) => format!("{} ({})", p, c),
                None => p.to_string(),
            })
        }).join(" | ");
        deps.push(dep);
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
