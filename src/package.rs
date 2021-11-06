use anyhow::Context;
use std::path::{Path, PathBuf};
use structopt::{clap::crate_version, StructOpt};

use crate::config::{parse_config, Config};
use crate::crates::CrateInfo;
use crate::debian::{self, DebInfo};
use crate::errors::Result;
use crate::util;

pub struct PackageProcess {
    // below state is filled in during init
    pub crate_info: CrateInfo,
    pub deb_info: DebInfo,
    pub config_path: Option<PathBuf>,
    pub config: Config,
    // below state is filled in during the process
    pub pkg_srcdir: Option<PathBuf>,
    pub source_modified: Option<bool>,
    pub tempdir: Option<tempfile::TempDir>,
    pub orig_tar_gz: Option<PathBuf>,
}

#[derive(Debug, Clone, StructOpt)]
pub struct PackageInitArgs {
    /// Name of the crate to package.
    pub crate_name: String,
    /// Version of the crate to package; may contain dependency operators.
    pub version: Option<String>,
    /// TOML file providing package-specific options.
    #[structopt(long)]
    pub config: Option<String>,
}

#[derive(Debug, Clone, StructOpt)]
pub struct PackageExtractArgs {
    /// Output directory.
    #[structopt(long)]
    pub directory: Option<String>,
}

#[derive(Debug, Clone, StructOpt)]
pub struct PackageFinishArgs {
    /// Assume the changelog is already bumped, and leave it alone.
    #[structopt(long)]
    pub changelog_ready: bool,
    /// Guess extra values for d/copyright. Might be slow.
    #[structopt(long)]
    pub copyright_guess_harder: bool,
    /// Don't write back hint files or d/changelog to the source overlay directory.
    #[structopt(long)]
    pub no_overlay_write_back: bool,
}

impl PackageProcess {
    pub fn new(init_args: PackageInitArgs) -> Result<Self> {
        let crate_name = &init_args.crate_name;
        let version = init_args.version.as_deref();
        let config = init_args.config.as_deref();

        let (config_path, config) = match config {
            Some(p) => {
                let path = Path::new(p);
                let config = parse_config(path).context("failed to parse debcargo.toml")?;
                (Some(path.to_path_buf()), config)
            }
            None => (None, Config::default()),
        };
        let crate_path = config.crate_src_path(config_path.as_deref());

        log::info!("preparing crate info");
        let mut crate_info = match crate_path {
            Some(p) => CrateInfo::new_with_local_crate(crate_name, version, &p)?,
            None => CrateInfo::new(crate_name, version)?,
        };
        crate_info.set_includes_excludes(config.orig_tar_excludes(), config.orig_tar_whitelist());
        let deb_info = DebInfo::new(
            crate_name,
            &crate_info,
            crate_version!(),
            config.semver_suffix,
        );

        Ok(Self {
            crate_info,
            deb_info,
            config_path,
            config,
            pkg_srcdir: None,
            source_modified: None,
            tempdir: None,
            orig_tar_gz: None,
        })
    }

    pub fn extract(&mut self, extract: PackageExtractArgs) -> Result<()> {
        assert!(self.pkg_srcdir.is_none());
        assert!(self.source_modified.is_none());
        let Self {
            crate_info,
            deb_info,
            ..
        } = self;
        // vars read; begin stage

        let pkg_srcdir = Path::new(
            extract
                .directory
                .as_deref()
                .unwrap_or_else(|| deb_info.package_source_dir()),
        )
        .to_path_buf();

        log::info!("extracting crate");
        let source_modified = crate_info.extract_crate(&pkg_srcdir)?;

        // stage finished; set vars
        self.pkg_srcdir = Some(pkg_srcdir);
        self.source_modified = Some(source_modified);
        Ok(())
    }

    pub fn apply_overrides(&mut self) -> Result<()> {
        assert!(self.tempdir.is_none());
        let Self {
            crate_info,
            config_path,
            config,
            pkg_srcdir,
            ..
        } = self;
        let pkg_srcdir = pkg_srcdir.as_ref().unwrap();
        // vars read; begin stage

        log::info!("applying overlay and patches");
        let tempdir = debian::apply_overlay_and_patches(
            crate_info,
            config_path.as_deref(),
            config,
            pkg_srcdir,
        )?;

        // stage finished; set vars
        self.tempdir = Some(tempdir);
        Ok(())
    }

    pub fn finish(&mut self, finish_args: PackageFinishArgs) -> Result<()> {
        assert!(self.orig_tar_gz.is_none());
        let Self {
            crate_info,
            deb_info,
            config_path,
            config,
            pkg_srcdir,
            source_modified,
            tempdir,
            ..
        } = self;
        let pkg_srcdir = pkg_srcdir.as_ref().unwrap();
        let source_modified = source_modified.as_ref().unwrap();
        let tempdir = tempdir.as_ref().unwrap();
        // vars read; begin stage

        log::info!("preparing orig tarball");
        let orig_tar_gz = pkg_srcdir
            .parent()
            .unwrap()
            .join(deb_info.orig_tarball_path());
        debian::prepare_orig_tarball(crate_info, &orig_tar_gz, *source_modified, pkg_srcdir)?;

        log::info!("preparing debian folder");
        debian::prepare_debian_folder(
            crate_info,
            deb_info,
            config_path.as_deref(),
            config,
            pkg_srcdir,
            tempdir,
            finish_args.changelog_ready,
            finish_args.copyright_guess_harder,
            !finish_args.no_overlay_write_back,
        )?;

        // stage finished; set vars
        self.orig_tar_gz = Some(orig_tar_gz);
        Ok(())
    }

    pub fn post_package_checks(&self) -> Result<()> {
        let Self {
            config_path,
            config,
            pkg_srcdir,
            orig_tar_gz,
            ..
        } = self;
        let pkg_srcdir = pkg_srcdir.as_ref().unwrap();
        let orig_tar_gz = orig_tar_gz.as_ref().unwrap();

        let curdir = std::env::current_dir()?;
        debcargo_info!(
            concat!("Package Source: {}\n", "Original Tarball for package: {}\n"),
            util::rel_p(pkg_srcdir, &curdir),
            util::rel_p(orig_tar_gz, &curdir)
        );
        let fixmes = util::lookup_fixmes(pkg_srcdir.join("debian").as_path())?;
        if !fixmes.is_empty() {
            debcargo_warn!("FIXME found in the following files.");
            for f in fixmes {
                if util::hint_file_for(&f).is_some() {
                    debcargo_warn!("\t(•) {}", util::rel_p(&f, &curdir));
                } else {
                    debcargo_warn!("\t •  {}", util::rel_p(&f, &curdir));
                }
            }
            debcargo_warn!("");
            debcargo_warn!("To fix, try combinations of the following: ");
            match config_path.as_deref() {
                None => debcargo_warn!("\t •  Write a config file and use it with --config"),
                Some(c) => {
                    debcargo_warn!("\t •  Add or edit overrides in your config file:");
                    debcargo_warn!("\t    {}", util::rel_p(c, &curdir));
                }
            };
            match config.overlay_dir(config_path.as_deref()) {
                None => debcargo_warn!("\t •  Create an overlay directory and add it to your config file with overlay = \"/path/to/overlay\""),
                Some(p) => {
                    debcargo_warn!("\t •  Add or edit files in your overlay directory:");
                    debcargo_warn!("\t    {}", util::rel_p(&p, &curdir));
                }
            }
        }
        Ok(())
    }
}
