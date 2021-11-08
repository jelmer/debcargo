use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::io::{BufRead, BufReader, Error};
use std::iter::Iterator;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::Command;

use walkdir::WalkDir;

pub const HINT_SUFFIX: &str = ".debcargo.hint";

pub fn hint_file_for(file: &Path) -> Option<&Path> {
    let file = file.as_os_str().as_bytes();
    if file.len() >= HINT_SUFFIX.len()
        && &file[file.len() - HINT_SUFFIX.len()..] == HINT_SUFFIX.as_bytes()
    {
        Some(Path::new(OsStr::from_bytes(
            &file[..file.len() - HINT_SUFFIX.len()],
        )))
    } else {
        None
    }
}

pub fn lookup_fixmes(srcdir: &Path) -> Result<BTreeSet<PathBuf>, Error> {
    let mut fixmes = BTreeSet::new();
    for entry in WalkDir::new(srcdir) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let file = fs::File::open(entry.path())?;
            let reader = BufReader::new(file);
            // If we find one FIXME we break the loop and check next file. Idea
            // is only to find files with FIXME strings in it.
            for line in reader.lines().flatten() {
                if line.contains("FIXME") {
                    fixmes.insert(entry.path().to_path_buf());
                    break;
                }
            }
        }
    }

    // ignore hint files whose non-hint partners exists and don't have a FIXME
    let fixmes = fixmes
        .iter()
        .filter(|f| match hint_file_for(f) {
            Some(ff) => fixmes.contains(ff) || !ff.exists(),
            None => true,
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    Ok(fixmes)
}

pub fn rel_p<'a>(path: &'a Path, base: &'a Path) -> &'a str {
    path.strip_prefix(base).unwrap_or(path).to_str().unwrap()
}

pub fn copy_tree(oldtree: &Path, newtree: &Path) -> Result<(), Error> {
    for entry in WalkDir::new(oldtree) {
        let entry = entry?;
        if entry.depth() == 0 {
            continue;
        }
        let oldpath = entry.path();
        let newpath = newtree.join(oldpath.strip_prefix(&oldtree).unwrap());
        let ftype = entry.file_type();
        if ftype.is_dir() {
            fs::create_dir(newpath)?;
        } else if ftype.is_file() {
            fs::copy(oldpath, newpath)?;
        } else if ftype.is_symlink() {
            symlink(fs::read_link(oldpath)?, newpath)?;
        }
    }
    Ok(())
}

pub fn vec_opt_iter<'a, T>(option: Option<&'a Vec<T>>) -> impl Iterator<Item = &T> + 'a {
    option.into_iter().flat_map(|v| v.iter())
}

pub fn expect_success(cmd: &mut Command, err: &str) {
    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                panic!("{}", err);
            }
        }
        Err(e) => {
            panic!("{}", e);
        }
    }
}

pub(crate) fn traverse_depth<'a>(
    map: &BTreeMap<&'a str, Vec<&'a str>>,
    key: &'a str,
) -> Vec<&'a str> {
    let mut x = Vec::new();
    if let Some(pp) = (*map).get(key) {
        x.extend(pp);
        for p in pp {
            x.extend(traverse_depth(map, p));
        }
    }
    x
}

/// Get a value that might be set at a key or any of its ancestor keys,
/// whichever is closest. Error if there are conflicting definitions.
#[allow(clippy::type_complexity)]
pub(crate) fn get_transitive_val<
    'a,
    P: Fn(K) -> Option<&'a Vec<K>>,
    F: Fn(K) -> Option<V>,
    K: 'a + Ord + Copy,
    V: Eq + Ord,
>(
    getparents: &'a P,
    f: &F,
    key: K,
) -> Result<Option<V>, (K, Vec<(K, V)>)> {
    let here = f(key);
    if here.is_some() {
        // value overrides anything from parents
        Ok(here)
    } else {
        let mut candidates = Vec::new();
        for par in vec_opt_iter(getparents(key)) {
            if let Some(v) = get_transitive_val(getparents, f, *par)? {
                candidates.push((*par, v))
            }
        }
        if candidates.is_empty() {
            Ok(None) // here is None
        } else {
            let mut values = candidates.iter().map(|(_, v)| v).collect::<Vec<_>>();
            values.sort();
            values.dedup();
            if values.len() == 1 {
                Ok(candidates.pop().map(|(_, v)| v))
            } else {
                Err((key, candidates)) // handle conflict
            }
        }
    }
}

pub fn graph_from_succ<V, FV, FL, E>(
    seed: impl IntoIterator<Item = V>,
    succ: &mut FV,
    log: &mut FL,
) -> Result<BTreeMap<V, BTreeSet<V>>, E>
where
    V: Ord + Clone,
    FV: FnMut(&V) -> Result<(Vec<V>, Vec<V>), E>,
    FL: FnMut(&VecDeque<V>, &BTreeMap<V, BTreeSet<V>>) -> Result<(), E>,
{
    let mut seen = BTreeSet::from_iter(seed);
    let mut graph = BTreeMap::new();
    let mut remain = VecDeque::from_iter(seen.iter().cloned());
    while let Some(v) = remain.pop_front() {
        log(&remain, &graph)?;
        let (hard, soft) = succ(&v)?;
        for v_ in hard.iter().chain(soft.iter()) {
            if !seen.contains(v_) {
                seen.insert(v_.clone());
                remain.push_back(v_.clone());
            }
        }
        graph.insert(v, BTreeSet::from_iter(hard));
    }
    // TODO: if there is an error then pass the graph to an error handler
    Ok(graph)
}

pub fn succ_proj<S, T, F>(succ: &BTreeMap<S, BTreeSet<S>>, proj: F) -> BTreeMap<T, BTreeSet<T>>
where
    F: Fn(&S) -> T,
    S: Ord,
    T: Ord + Clone,
{
    let mut succ_proj: BTreeMap<T, BTreeSet<T>> = BTreeMap::new();
    for (s, ss) in succ {
        let e = succ_proj.entry(proj(s)).or_default();
        for s_ in ss {
            e.insert(proj(s_));
        }
    }
    succ_proj
}

pub fn succ_to_pred<V>(succ: &BTreeMap<V, BTreeSet<V>>) -> BTreeMap<V, BTreeSet<V>>
where
    V: Ord + Clone,
{
    let mut pred: BTreeMap<V, BTreeSet<V>> = BTreeMap::new();
    for (v, vv) in succ {
        for v_ in vv {
            pred.entry(v_.clone()).or_default().insert(v.clone());
        }
    }
    pred
}

pub fn topo_sort<V>(
    seed: impl IntoIterator<Item = V>,
    succ: BTreeMap<V, BTreeSet<V>>,
    mut pred: BTreeMap<V, BTreeSet<V>>,
) -> Result<Vec<V>, BTreeMap<V, BTreeSet<V>>>
where
    V: Ord + Clone,
{
    let empty = BTreeSet::new();
    let mut remain = VecDeque::from_iter(seed);
    let mut sort = Vec::new();
    while let Some(v) = remain.pop_front() {
        sort.push(v.clone());
        for v_ in succ.get(&v).unwrap_or(&empty) {
            let par = pred.entry(v_.clone()).or_default();
            par.remove(&v);
            if par.is_empty() {
                remain.push_back(v_.clone());
            }
        }
    }
    pred.retain(|_, v| !v.is_empty());
    if !pred.is_empty() {
        Err(pred)
    } else {
        Ok(sort)
    }
}
