use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use cargo::core::{Dependency, PackageId};
use clap::arg_enum;

use crate::crates::{CrateDepInfo, CrateInfo};
use crate::errors::Result;
use crate::util;

arg_enum! {
    #[derive(Debug, Copy, Clone)]
    pub enum ResolveType {
        CargoBinaryUpstream,
        DebianBinaryUnstable,
        DebianSourceTesting,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct PackageIdFeat(PackageId, &'static str);

fn get_build_deps(crate_dep_info: &CrateDepInfo, feature: &str) -> Result<Vec<Dependency>> {
    if feature == "@" {
        let deps = crate_dep_info
            .iter()
            .map(|(_, v)| v.1.iter())
            .flatten()
            .collect::<HashSet<_>>();
        Ok(deps.into_iter().cloned().collect::<Vec<_>>())
    } else {
        // note: please keep this logic in sync with prepare_debian_control
        let (_, default_deps) = super::transitive_deps(crate_dep_info, feature);
        Ok(default_deps)
    }
}

fn dep_features(dep: &Dependency, resolve_type: ResolveType) -> Vec<&'static str> {
    use ResolveType::*;
    match resolve_type {
        CargoBinaryUpstream => {
            let mut feats = dep
                .features()
                .iter()
                .map(|x| x.as_str())
                .collect::<Vec<_>>();
            if dep.uses_default_features() {
                feats.push("default")
            }
            feats
        }
        DebianBinaryUnstable => {
            // debian source packages unconditionally build-depend on the default feature set;
            // see super::prepare_debian_control
            let mut feats = dep
                .features()
                .iter()
                .map(|x| x.as_str())
                .collect::<Vec<_>>();
            feats.push("default");
            feats
        }
        DebianSourceTesting => {
            // debian source packages produce binary packages that represent all features.
            // However instead of returning all features explicitly, we return @ here which
            // is special-cased in get_build_deps; this avoids resolving lots of duplicates
            vec!["@"]
        }
    }
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

// FIXME: add the ability to apply our Cargo.toml patches that reduce the
// build-dependency set. this could help prevent cycles.
pub fn build_order(
    crate_name: &str,
    version: Option<&str>,
    resolve_type: ResolveType,
) -> Result<Vec<PackageId>> {
    let mut infos = BTreeMap::new();
    let seed = CrateInfo::new(crate_name, version)?;
    let seed_id = insert_info(&mut infos, seed);

    let mut next = |idf: &PackageIdFeat| -> Result<Vec<PackageIdFeat>> {
        let crate_info = infos
            .get(&idf.0)
            .expect("build_order next called without crate info");
        let mut deps = Vec::new();
        for dep in get_build_deps(&crate_info.1, idf.1)? {
            // we might end up resolving the same crate-version several times;
            // this is expected, since different dependencies (with different
            // version ranges) might resolve into the same crate-version
            let info = CrateInfo::new_from_dependency(&dep, false)?;
            let id = insert_info(&mut infos, info);
            for f in dep_features(&dep, resolve_type) {
                deps.push(PackageIdFeat(id, f));
            }
        }
        Ok(deps)
    };
    let mut i = 0;
    let mut log = |remaining: &VecDeque<_>, graph: &BTreeMap<_, _>| {
        i += 1;
        if i % 16 == 0 {
            log::info!(
                "build-order: done: {}, todo: {}",
                graph.len(),
                remaining.len()
            );
        }
        Ok(())
    };

    use ResolveType::*;
    let seed_idf = PackageIdFeat(
        seed_id,
        match resolve_type {
            CargoBinaryUpstream => "",
            DebianBinaryUnstable => "default",
            DebianSourceTesting => "@",
        },
    );
    let succ_with_features = util::graph_from_succ([seed_idf], &mut next, &mut log)?;
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
        log::info!(
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
