use std::fs;
use std::ffi::OsStr;
use std::io::Error;
use std::path::Path;
use std::os::unix::fs::symlink;
use std::os::unix::ffi::OsStrExt;

use walkdir;


pub const HINT_SUFFIX: &'static str = ".debcargo.hint";


pub fn is_hint_file(file: &OsStr) -> bool {
    let file = file.as_bytes();
    file.len() >= HINT_SUFFIX.len() &&
        &file[file.len()-HINT_SUFFIX.len()..] == HINT_SUFFIX.as_bytes()
}


pub fn copy_tree(oldtree: &Path, newtree: &Path) -> Result<(), Error> {
    for entry in walkdir::WalkDir::new(oldtree) {
        let entry = entry?;
        if entry.depth() == 0 { continue }
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
