/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fmt::Write;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use walkdir::WalkDir;

use crate::buckify::relative_path;
use crate::cargo::Manifest;
use crate::cargo::ManifestTarget;
use crate::fixups::buildscript::BuildscriptFixup;
use crate::fixups::buildscript::BuildscriptFixups;
use crate::index::Index;
use crate::index::ResolvedDep;
use crate::platform::PlatformExpr;

/// Top-level fixup config file (correspondins to a fixups.toml)
#[derive(Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixupConfigFile {
    /// Limit an exposed crate's `alias`'s `visibility` to this.
    /// This only has an effect for top-level crates. Exposed crates
    /// by default get `visibility = ["PUBLIC"]`. Sometimes you want to
    /// discourage use of some crate by limiting its visibility.
    #[serde(default, rename = "visibility")]
    pub custom_visibility: Option<Vec<String>>,

    /// Omit a target
    #[serde(default)]
    pub omit_targets: BTreeSet<String>,

    /// Skip precise srcs detection and fallback to `**/*.rs`.
    /// Overrides the global config `precise_srcs` for this crate.
    /// This is useful for pathologically large crates where
    /// src detection dominates buckification (e.g. the `windows` crate).
    pub precise_srcs: Option<bool>,

    /// If the crate is generating a cdylib which is intended to be
    /// a Python extension module, set this to give the module name.
    /// This is passed as a `python_ext` parameter on the `rust_library`
    /// rule so it can be mapped to the right underlying rule.
    pub python_ext: Option<String>,

    /// Common config
    #[serde(flatten)]
    base: FixupConfig,

    /// Platform-specific configs
    #[serde(default)]
    platform_fixup: BTreeMap<PlatformExpr, FixupConfig>,
}

impl FixupConfigFile {
    /// Generate a template for a fixup.toml as a starting point. This includes the list of
    /// buildscript dependencies, as a clue as to what the buildscript is trying to do.
    pub fn template<'meta>(
        third_party_path: &Path,
        index: &'meta Index<'meta>,
        package: &'meta Manifest,
        target: &'meta ManifestTarget,
    ) -> Self {
        if !target.kind_custom_build() {
            return Default::default();
        }

        let relpath = relative_path(third_party_path, &target.src_path);
        let mut msg = format!(
            "Unresolved build script at {}. Dependencies:",
            relpath.display()
        );
        for ResolvedDep { package, .. } in index.resolved_deps_for_target(package, target) {
            let _ = write!(&mut msg, "\n    {}", package);
        }

        let buildscript = vec![BuildscriptFixup::Unresolved(msg)];

        FixupConfigFile {
            base: FixupConfig {
                buildscript: BuildscriptFixups(buildscript),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn base(&self, version: &semver::Version) -> Option<&FixupConfig> {
        if self.base.version_applies(version) {
            Some(&self.base)
        } else {
            None
        }
    }

    pub fn platform_configs<'a>(
        &'a self,
        version: &'a semver::Version,
    ) -> impl Iterator<Item = (&PlatformExpr, &FixupConfig)> + 'a {
        self.platform_fixup
            .iter()
            .filter(move |(_, cfg)| cfg.version_applies(version))
    }

    pub fn configs<'a>(
        &'a self,
        version: &'a semver::Version,
    ) -> impl Iterator<Item = (Option<&PlatformExpr>, &FixupConfig)> + 'a {
        self.base(version)
            .into_iter()
            .map(|base| (None, base))
            .chain(
                self.platform_configs(version)
                    .map(|(plat, cfg)| (Some(plat), cfg)),
            )
    }
}

#[derive(Debug, Deserialize, Default, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixupConfig {
    /// Versions this fixup applies to,
    pub version: Option<semver::VersionReq>,
    /// Extra src globs, rooted in manifest dir for package
    #[serde(default)]
    pub extra_srcs: Vec<String>,
    /// Dirs with contents that are referenced in src files.
    /// Relative to manifest dir for package.
    #[serde(default)]
    pub src_referenced_dirs: Vec<PathBuf>,
    /// Extra flags for rustc
    #[serde(default)]
    pub rustc_flags: Vec<String>,
    /// Extra configs
    #[serde(default)]
    pub cfgs: BTreeSet<String>,
    /// Extra features
    #[serde(default)]
    pub features: BTreeSet<String>,
    /// Features to forcably omit. This doesn't change dependency
    /// resolution, just what the targets are compiled with.
    #[serde(default)]
    pub omit_features: BTreeSet<String>,
    /// Additional Buck dependencies
    #[serde(default)]
    pub extra_deps: BTreeSet<String>,
    /// Omit Cargo dependencies - just bare crate name
    #[serde(default)]
    pub omit_deps: BTreeSet<String>,
    /// Add Cargo environment
    #[serde(default)]
    pub cargo_env: bool,
    /// Path relative to fixups_dir with overlay filesystem
    /// Files in overlay logically add to or replace files in
    /// manifest dir, and therefore have the same directory
    /// structure.
    pub overlay: Option<PathBuf>,
    /// Rust binary link style (how dependencies should be linked)
    pub link_style: Option<String>,
    /// Rust library preferred linkage (how dependents should link you)
    pub preferred_linkage: Option<String>,

    // Table/map-like values must come after everything else
    /// Additional env variables
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// How to handle a build-script, if present
    #[serde(default)]
    pub buildscript: BuildscriptFixups,
    /// Extra mapped srcs
    #[serde(default)]
    pub extra_mapped_srcs: BTreeMap<String, PathBuf>,
}

impl FixupConfig {
    /// Return set of overlay files, relative to the overlay dir (and therefore
    /// relative to manifest dir).
    pub fn overlay_files(&self, fixup_dir: &Path) -> Result<HashSet<PathBuf>> {
        let files = match &self.overlay {
            Some(overlay) => {
                let overlay_dir = fixup_dir.join(overlay);
                WalkDir::new(&overlay_dir)
                    .into_iter()
                    .filter_map(|ent| ent.ok())
                    .filter(|ent| ent.path().is_file())
                    .map(|ent| relative_path(&overlay_dir, ent.path()))
                    .collect()
            }
            None => HashSet::new(),
        };

        Ok(files)
    }

    /// Return true if config applies to given version
    pub fn version_applies(&self, ver: &semver::Version) -> bool {
        self.version.as_ref().map_or(true, |req| req.matches(ver))
    }
}
