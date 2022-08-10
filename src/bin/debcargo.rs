use ansi_term::Colour::Red;
use clap::{crate_version, AppSettings, StructOpt};

use debcargo::crates::CrateInfo;
use debcargo::debian::DebInfo;
use debcargo::errors::Result;
use debcargo::package::*;
use debcargo::{
    build_order::{build_order, BuildOrderArgs},
    crates::invalidate_crates_io_cache,
};

#[derive(Debug, Clone, StructOpt)]
#[structopt(name = "debcargo", about = "Package Rust crates for Debian.")]
enum Opt {
    /// Update the user's default crates.io index, outside of a workspace.
    Update,
    /// Print the Debian package name for a crate.
    DebSrcName {
        /// Name of the crate to package.
        crate_name: String,
        /// Version of the crate to package; may contain dependency operators.
        /// If empty string, resolves to the latest version. If given here,
        /// i.e. not omitted then print the package name as if the config
        /// option semver_suffix was set to true.
        version: Option<String>,
    },
    /// Extract only a crate, without any other transformations.
    Extract {
        #[structopt(flatten)]
        init: PackageInitArgs,
        #[structopt(flatten)]
        extract: PackageExtractArgs,
    },
    /// Package a Rust crate for Debian.
    Package {
        #[structopt(flatten)]
        init: PackageInitArgs,
        #[structopt(flatten)]
        extract: PackageExtractArgs,
        #[structopt(flatten)]
        finish: PackageExecuteArgs,
    },
    /// Print the transitive dependencies of a package in topological order.
    BuildOrder {
        #[structopt(flatten)]
        args: BuildOrderArgs,
    },
}

#[test]
fn verify_app() {
    use clap::IntoApp;
    Opt::into_app().debug_assert()
}

fn real_main() -> Result<()> {
    let m = Opt::clap()
        .global_setting(AppSettings::ColoredHelp)
        .get_matches();
    use Opt::*;
    match Opt::from_clap(&m) {
        Update => invalidate_crates_io_cache(),
        DebSrcName {
            crate_name,
            version,
        } => {
            let crate_info = CrateInfo::new_with_update(&crate_name, version.as_deref(), false)?;
            let deb_info = DebInfo::new(&crate_info, crate_version!(), version.is_some());
            println!("{}", deb_info.package_name());
            Ok(())
        }
        Extract { init, extract } => {
            log::info!("preparing crate info");
            let mut process = PackageProcess::init(init)?;
            log::info!("extracting crate");
            process.extract(extract)?;
            Ok(())
        }
        Package {
            init,
            extract,
            finish,
        } => {
            log::info!("preparing crate info");
            let mut process = PackageProcess::init(init)?;
            log::info!("extracting crate");
            process.extract(extract)?;
            log::info!("applying overlay and patches");
            process.apply_overrides()?;
            log::info!("preparing orig tarball");
            process.prepare_orig_tarball()?;
            log::info!("preparing debian folder");
            process.prepare_debian_folder(finish)?;
            process.post_package_checks()
        }
        BuildOrder { args } => {
            let build_order = build_order(args)?;
            for v in &build_order {
                println!("{}", v);
            }
            Ok(())
        }
    }
}

fn main() {
    env_logger::init();
    if let Err(e) = real_main() {
        eprintln!("{}", Red.bold().paint(format!("debcargo failed: {:?}", e)));
        std::process::exit(1);
    }
}
