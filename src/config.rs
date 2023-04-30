/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! Global third-party config

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::de::value::MapAccessDeserializer;
use serde::de::Deserializer;
use serde::de::MapAccess;
use serde::de::Visitor;
use serde::Deserialize;

use crate::platform::PlatformConfig;
use crate::platform::PlatformName;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Path the config was read from
    #[serde(skip)]
    pub config_path: PathBuf,

    /// Default flags applied to all rules
    #[serde(default)]
    pub rustc_flags: Vec<String>,

    /// Platform-specific rustc flags
    #[serde(default)]
    pub platform_rustc_flags: BTreeMap<PlatformName, Vec<String>>,

    /// Try to compute a precise list of sources rather than using globbing
    #[serde(default)]
    pub precise_srcs: bool,

    /// List of glob patterns for filenames likely to contain license terms
    #[serde(default)]
    pub license_patterns: BTreeSet<String>,

    /// Generate fixup file templates when missing
    #[serde(default)]
    pub fixup_templates: bool,

    /// Fail buckify if there are unresolved fixups
    #[serde(default)]
    pub unresolved_fixup_error: bool,

    ///Provide additional information to resolve unresolved fixup errors
    #[serde(default)]
    pub unresolved_fixup_error_message: Option<String>,

    /// Path to buck cell root (if relative, relative to here)
    #[serde(default)]
    pub buck_cell_root: Option<PathBuf>,

    /// Include root package as top-level public target in Buck file
    #[serde(default)]
    pub include_top_level: bool,

    /// Emit metadata for each crate into Buck rules
    #[serde(default)]
    pub emit_metadata: bool,

    /// Use strict glob matching
    #[serde(default)]
    pub strict_globs: bool,

    #[serde(default)]
    pub cargo: CargoConfig,

    #[serde(default)]
    pub buck: BuckConfig,

    #[serde(
        default = "default_vendor_config",
        deserialize_with = "deserialize_vendor_config"
    )]
    pub vendor: Option<VendorConfig>,

    #[serde(default)]
    pub audit: AuditConfig,

    #[serde(default)]
    pub platform: HashMap<PlatformName, PlatformConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CargoConfig {
    /// Path to cargo executable. If set, then relative to this file
    #[serde(default)]
    pub cargo: Option<PathBuf>,
    /// Always version vendor directories (requires cargo with --versioned-dirs)
    #[serde(default)]
    pub versioned_dirs: bool,
    /// Support Cargo's unstable "artifact dependencies" functionality, RFC 3028.
    #[serde(default)]
    pub bindeps: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuckConfig {
    /// Name of BUCK file
    #[serde(default = "default_buck_file_name")]
    pub file_name: String,
    /// Banner for the top of all generated bzl files, namely BUCK and METADATA.bzl
    #[serde(default)]
    pub generated_file_header: String,
    /// Front matter for the generated BUCK file
    #[serde(default)]
    pub buckfile_imports: String,

    /// Rule name for alias
    #[serde(default = "default_alias")]
    pub alias: String,
    /// Rule name for http_archive
    #[serde(default = "default_http_archive")]
    pub http_archive: String,
    /// Rule name for rust_library
    #[serde(default = "default_rust_library")]
    pub rust_library: String,
    /// Rule name for rust_binary
    #[serde(default = "default_rust_binary")]
    pub rust_binary: String,
    /// Rule name for cxx_library
    #[serde(default = "default_cxx_library")]
    pub cxx_library: String,
    /// Rule name for prebuilt_cxx_library
    #[serde(default = "default_prebuilt_cxx_library")]
    pub prebuilt_cxx_library: String,
    /// Rule name for the rust_binary of a build script
    pub buildscript_binary: Option<String>,
    /// Rule name for a build script invocation
    #[serde(default = "default_buildscript_genrule")]
    pub buildscript_genrule: String,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VendorConfig {
    /// List of .gitignore files to use to filter checksum files, relative to
    /// this config file.
    #[serde(default)]
    pub gitignore_checksum_exclude: HashSet<PathBuf>,
    /// Set of globs to remove from Cargo's checksun files in vendored dirs
    #[serde(default)]
    pub checksum_exclude: HashSet<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditConfig {
    /// List of package names to never attempt to autofix
    #[serde(default)]
    pub never_autofix: HashSet<String>,
}

fn default_buck_file_name() -> String {
    BuckConfig::default().file_name
}

fn default_alias() -> String {
    BuckConfig::default().alias
}

fn default_http_archive() -> String {
    BuckConfig::default().http_archive
}

fn default_rust_library() -> String {
    BuckConfig::default().rust_library
}

fn default_rust_binary() -> String {
    BuckConfig::default().rust_binary
}

fn default_cxx_library() -> String {
    BuckConfig::default().cxx_library
}

fn default_prebuilt_cxx_library() -> String {
    BuckConfig::default().prebuilt_cxx_library
}

fn default_buildscript_genrule() -> String {
    BuckConfig::default().buildscript_genrule
}

fn default_vendor_config() -> Option<VendorConfig> {
    Some(VendorConfig::default())
}

fn deserialize_vendor_config<'de, D>(deserializer: D) -> Result<Option<VendorConfig>, D::Error>
where
    D: Deserializer<'de>,
{
    struct VendorConfigVisitor;

    impl<'de> Visitor<'de> for VendorConfigVisitor {
        type Value = Option<VendorConfig>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("[vendor] section, or `vendor = false`")
        }

        fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            // `vendor = true`: default configuration with vendoring.
            // `vendor = false`: do not vendor.
            Ok(value.then(VendorConfig::default))
        }

        fn visit_map<M>(self, map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            VendorConfig::deserialize(MapAccessDeserializer::new(map)).map(Some)
        }
    }

    deserializer.deserialize_any(VendorConfigVisitor)
}

impl Default for BuckConfig {
    fn default() -> Self {
        BuckConfig {
            file_name: "BUCK".to_string(),
            generated_file_header: String::new(),
            buckfile_imports: String::new(),

            alias: "alias".to_string(),
            http_archive: "http_archive".to_string(),
            rust_library: "rust_library".to_string(),
            rust_binary: "rust_binary".to_string(),
            cxx_library: "cxx_library".to_string(),
            prebuilt_cxx_library: "prebuilt_cxx_library".to_string(),
            buildscript_binary: None,
            buildscript_genrule: "buildscript_run".to_string(),
        }
    }
}

pub fn read_config(dir: &Path) -> Result<Config> {
    let reindeer_toml = dir.join("reindeer.toml");
    let mut config = try_read_config(&reindeer_toml)?;
    config.config_path = dir.to_path_buf();
    Ok(config)
}

fn try_read_config(path: &Path) -> Result<Config> {
    let file = match fs::read(path) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Config::default()),
        Err(err) => return Err(err).context(format!("Failed to read config {}", path.display())),
    };

    let config: Config =
        toml::de::from_slice(&file).context(format!("Failed to parse {}", path.display()))?;

    log::debug!("Read config {:#?}", config);

    Ok(config)
}
