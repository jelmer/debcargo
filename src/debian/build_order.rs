use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use cargo::core::{Dependency, PackageId};
use structopt::clap::arg_enum;

use crate::crates::{CrateDepInfo, CrateInfo};
use crate::errors::Result;
use crate::util;

arg_enum! {
    #[derive(Debug, Copy, Clone)]
    pub enum ResolveType {
        BinaryForDebianUnstable,
        SourceForDebianTesting,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PackageIdFeat(PackageId, &'static str);

// First result: if somebody build-depends on us, what do they first need to build?
// Second result: what other packages need to go into Debian Testing before us?
fn get_build_deps(
    crate_dep_info: &CrateDepInfo,
    package: &PackageIdFeat,
    resolve_type: ResolveType,
    emulate_collapse_features: bool,
) -> Result<(Vec<Dependency>, Vec<Dependency>)> {
    let all_deps = crate_dep_info
        .iter()
        .map(|(_, v)| v.1.iter())
        .flatten()
        .cloned()
        .collect::<HashSet<_>>();
    let feature_deps: HashSet<Dependency> =
        HashSet::from_iter(super::transitive_deps(crate_dep_info, package.1).1);
    // FIXME: read crate config, including Cargo.toml patches
    // note: please keep the below logic in sync with prepare_debian_control
    let additional_deps = if emulate_collapse_features {
        // FIXME: if crate config collapse_features is on, also branch here
        all_deps.clone()
    } else {
        // FIXME: if build_depends_features is an override, use that instead of "default"
        // TODO: also deprecate build_depends_excludes
        HashSet::from_iter(super::transitive_deps(crate_dep_info, "default").1)
    };
    let hard_deps = feature_deps
        .union(&additional_deps)
        .cloned()
        .collect::<Vec<_>>();
    use ResolveType::*;
    match resolve_type {
        BinaryForDebianUnstable => Ok((hard_deps, vec![])),
        SourceForDebianTesting => {
            let mut soft_deps = all_deps;
            for h in hard_deps.iter() {
                soft_deps.remove(h);
            }
            Ok((hard_deps, soft_deps.into_iter().collect::<Vec<_>>()))
        }
    }
}

fn dep_features(dep: &Dependency) -> Vec<&'static str> {
    let mut feats = dep
        .features()
        .iter()
        .map(|x| x.as_str())
        .collect::<Vec<_>>();
    if dep.uses_default_features() {
        feats.push("default")
    }
    feats.push(""); // bare-bones library with no features
    feats
}

fn insert_info(
    infos: &mut BTreeMap<PackageId, (CrateInfo, CrateDepInfo)>,
    info: CrateInfo,
) -> PackageId {
    let id = info.package_id();
    let dep_info = info.all_dependencies_and_features();
    infos.insert(id, (info, dep_info));
    id
}

fn ensure_info(
    package: &PackageId,
    dependency: &Dependency,
    infos: &mut BTreeMap<PackageId, (CrateInfo, CrateDepInfo)>,
    cache: &mut HashMap<Dependency, PackageId>,
) -> Result<PackageId> {
    match cache.get(dependency) {
        Some(id) => Ok(*id),
        None => {
            let info = CrateInfo::new_from_dependency(Some(*package), dependency, false)?;
            Ok(insert_info(infos, info))
        }
    }
}

// FIXME: add the ability to apply our Cargo.toml patches that reduce the
// build-dependency set. this could help prevent cycles.
pub fn build_order(
    crate_name: &str,
    version: Option<&str>,
    resolve_type: ResolveType,
    emulate_collapse_features: bool,
) -> Result<Vec<PackageId>> {
    let mut infos = BTreeMap::new();
    let mut cache = HashMap::new();
    let seed = CrateInfo::new(crate_name, version)?;
    let seed_id = insert_info(&mut infos, seed);

    let mut next = |idf: &PackageIdFeat| -> Result<(Vec<PackageIdFeat>, Vec<PackageIdFeat>)> {
        let crate_info = infos
            .get(&idf.0)
            .expect("build_order next called without crate info");
        let (hard, soft) =
            get_build_deps(&crate_info.1, idf, resolve_type, emulate_collapse_features)?;
        // note: we might resolve the same crate-version several times;
        // this is expected, since different dependencies (with different
        // version ranges) might resolve into the same crate-version
        let mut hard_p = Vec::new();
        for dep in hard {
            let id = ensure_info(&idf.0, &dep, &mut infos, &mut cache)?;
            for f in dep_features(&dep) {
                hard_p.push(PackageIdFeat(id, f));
            }
        }
        let mut soft_p = Vec::new();
        for dep in soft {
            let id = ensure_info(&idf.0, &dep, &mut infos, &mut cache)?;
            for f in dep_features(&dep) {
                soft_p.push(PackageIdFeat(id, f));
            }
        }
        Ok((hard_p, soft_p))
    };
    let mut i = 0;
    let mut log = |remaining: &VecDeque<_>, graph: &BTreeMap<_, _>| {
        i += 1;
        if i % 16 == 0 {
            debcargo_info!(
                "build-order: done: {}, todo: {}",
                graph.len(),
                remaining.len()
            );
        }
        Ok(())
    };

    let succ_with_features =
        util::graph_from_succ([PackageIdFeat(seed_id, "")], &mut next, &mut log)?;
    log::trace!("succ_with_features: {:#?}", succ_with_features);

    let succ = util::succ_proj(&succ_with_features, |x| x.0);
    let pred = util::succ_to_pred(&succ);
    let roots = succ
        .iter()
        .filter_map(|(k, v)| if v.is_empty() { Some(*k) } else { None })
        .collect::<BTreeSet<_>>();
    // swap pred/succ for call to topo_sort since we want reverse topo order
    let build_order = match util::topo_sort(roots, pred.clone(), succ.clone()) {
        Ok(r) => r,
        Err(remain) => {
            log::error!(
                "topo_sort got cyclic graph: {:#?}",
                remain
                    .into_iter()
                    .map(|(k, vv)| (
                        k.to_string(),
                        vv.into_iter().map(|v| v.to_string()).collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
            debcargo_bail!(
                "topo_sort got cyclic graph; you'll need to patch the crate(s) to break the cycle."
            )
        }
    };

    // sanity check
    for p in build_order.iter() {
        if infos.remove(p).is_none() {
            log::error!("extra package in build-order not in infos: {}", p);
        }
    }
    for (p, _) in infos {
        log::error!(
            "leftover infos not used in build-order: {}, succ: {:#?}, pred: {:#?}",
            p,
            succ.get(&p)
                .map(|x| x.iter())
                .into_iter()
                .flatten()
                .map(|x| x.to_string())
                .collect::<Vec<_>>(),
            pred.get(&p)
                .map(|x| x.iter())
                .into_iter()
                .flatten()
                .map(|x| x.to_string())
                .collect::<Vec<_>>(),
        );
    }

    Ok(build_order)
}
