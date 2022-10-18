/*
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 *
 * This source code is licensed under the MIT license found in the
 * LICENSE file in the root directory of this source tree.
 */

//! Definitions of Buck-related types
//!
//! Model Buck rules in a rough way. Can definitely be improved.
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt;
use std::fmt::Display;
use std::io::Error;
use std::io::Write;
use std::path::PathBuf;

use semver::Version;
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::Deserialize;
use serde::Serialize;

use crate::collection::SetOrMap;
use crate::config::BuckConfig;
use crate::platform::PlatformConfig;
use crate::platform::PlatformExpr;
use crate::platform::PlatformName;
use crate::platform::PlatformPredicate;
use crate::platform::PredicateParseError;

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct RuleRef {
    target: RuleTarget,
    platform: Option<PlatformExpr>,
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum RuleTarget {
    Local(String),
    Abs(String),
}

impl RuleRef {
    pub fn local(target: String) -> Self {
        RuleRef {
            target: RuleTarget::Local(target),
            platform: None,
        }
    }

    pub fn abs(target: String) -> Self {
        RuleRef {
            target: RuleTarget::Abs(target),
            platform: None,
        }
    }

    pub fn with_platform(self, platform: Option<&PlatformExpr>) -> Self {
        RuleRef {
            target: self.target,
            platform: platform.cloned(),
        }
    }

    pub fn target(&self) -> &RuleTarget {
        &self.target
    }

    pub fn has_platform(&self) -> bool {
        self.platform.is_some()
    }

    /// Return true if one of the platform_configs applies to this rule. Always returns
    /// true if this dep has no platform constraint.
    pub fn filter(&self, platform_config: &PlatformConfig) -> Result<bool, PredicateParseError> {
        let res = match &self.platform {
            None => true,
            Some(cfg) => {
                let cfg = PlatformPredicate::parse(cfg)?;

                cfg.eval(platform_config)
            }
        };
        Ok(res)
    }
}

impl Serialize for RuleRef {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        self.target.serialize(ser)
    }
}

impl Display for RuleTarget {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            RuleTarget::Local(name) => write!(fmt, ":{}", name),
            RuleTarget::Abs(name) => write!(fmt, "{}", name),
        }
    }
}

impl Serialize for RuleTarget {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RuleTarget {
    fn deserialize<D>(deser: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deser)?;
        let res = if s.starts_with(':') {
            RuleTarget::Local(s)
        } else {
            // Should really check it contains "//"
            RuleTarget::Abs(s)
        };
        Ok(res)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub struct BuckPath(pub PathBuf);

impl Serialize for BuckPath {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        // Even on Windows we want to use forward slash paths
        match self.0.as_path().to_str() {
            Some(s) => s.replace('\\', "/").serialize(ser),
            None => Err(serde::ser::Error::custom(
                "path contains invalid UTF-8 characters",
            )),
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct Alias {
    pub name: String,
    pub actual: RuleRef,
    #[serde(rename = "visibility", serialize_with = "visibility")]
    pub public: bool,

    // Dummy map to make serde treat this struct as a map
    #[serde(skip_serializing, flatten)]
    pub _dummy: BTreeMap<(), ()>,
}

fn visibility<S: Serializer>(vis: &bool, ser: S) -> Result<S::Ok, S::Error> {
    if *vis { vec!["PUBLIC"] } else { vec![] }.serialize(ser)
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize)]
pub struct Common {
    pub name: String,
    #[serde(rename = "visibility", serialize_with = "visibility")]
    pub public: bool,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub licenses: BTreeSet<BuckPath>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub compatible_with: Vec<RuleRef>,
}

fn always<T>(_: &T) -> bool {
    true
}

// Rule attributes which could be platform-specific
#[derive(Debug, Default, Clone, Eq, PartialEq, Serialize, Ord, PartialOrd)]
pub struct PlatformRustCommon {
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub srcs: BTreeSet<BuckPath>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub mapped_srcs: BTreeMap<String, BuckPath>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rustc_flags: Vec<String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub features: BTreeSet<String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub deps: BTreeSet<RuleRef>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub named_deps: BTreeMap<String, RuleRef>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,

    // This isn't really "common" (Binaries only), but does need to be platform
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_style: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_linkage: Option<String>,

    // Dummy map to make serde treat this struct as a map
    #[serde(skip_serializing_if = "always", flatten)]
    pub _dummy: BTreeMap<(), ()>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Ord, PartialOrd)]
pub struct RustCommon {
    #[serde(flatten)]
    pub common: Common,
    #[serde(rename = "crate")]
    pub krate: String,
    #[serde(rename = "crate_root")]
    pub rootmod: BuckPath,
    pub edition: crate::cargo::Edition,
    // Platform-dependent
    #[serde(flatten)]
    pub base: PlatformRustCommon,

    // Platform-specific
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub platform: BTreeMap<PlatformName, PlatformRustCommon>,
}

fn is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RustLibrary {
    #[serde(flatten)]
    pub common: RustCommon,
    #[serde(skip_serializing_if = "is_false")]
    pub proc_macro: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub dlopen_enable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub python_ext: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct RustBinary {
    #[serde(flatten)]
    pub common: RustCommon,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct BuildscriptGenrule {
    pub name: String,
    pub buildscript_rule: RuleRef,
    pub package_name: String,
    pub version: Version,
    pub features: BTreeSet<String>,
    pub cfgs: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub path_env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct BuildscriptGenruleFilter {
    #[serde(flatten)]
    pub base: BuildscriptGenrule,
    pub outfile: String,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct BuildscriptGenruleSrcs {
    #[serde(flatten)]
    pub base: BuildscriptGenrule,
    pub files: BTreeSet<String>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub srcs: BTreeSet<BuckPath>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct CxxLibrary {
    #[serde(flatten)]
    pub common: Common,
    pub srcs: BTreeSet<BuckPath>,
    pub headers: BTreeSet<BuckPath>,
    #[serde(skip_serializing_if = "SetOrMap::is_empty")]
    pub exported_headers: SetOrMap<BuckPath>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub compiler_flags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub preprocessor_flags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub header_namespace: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub include_directories: Vec<BuckPath>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    pub deps: BTreeSet<RuleRef>,
    pub preferred_linkage: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct PrebuiltCxxLibrary {
    #[serde(flatten)]
    pub common: Common,
    pub static_lib: BuckPath,
}

#[derive(Debug)]
pub enum Rule {
    Alias(Alias),
    Binary(RustBinary),
    Library(RustLibrary),
    BuildscriptGenruleSrcs(BuildscriptGenruleSrcs),
    BuildscriptGenruleFilter(BuildscriptGenruleFilter),
    CxxLibrary(CxxLibrary),
    PrebuiltCxxLibrary(PrebuiltCxxLibrary),
}

impl Eq for Rule {}

impl PartialEq for Rule {
    fn eq(&self, other: &Self) -> bool {
        self.get_name().eq(other.get_name())
    }
}

impl PartialOrd for Rule {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Rule {
    fn cmp(&self, other: &Self) -> Ordering {
        self.get_name().cmp(other.get_name())
    }
}

impl Rule {
    pub fn get_name(&self) -> &str {
        match self {
            Rule::Alias(Alias { name, .. }) => name,
            Rule::Binary(RustBinary {
                common:
                    RustCommon {
                        common: Common { name, .. },
                        ..
                    },
                ..
            }) => name,
            Rule::Library(RustLibrary {
                common:
                    RustCommon {
                        common: Common { name, .. },
                        ..
                    },
                ..
            }) => name,
            Rule::BuildscriptGenruleSrcs(BuildscriptGenruleSrcs {
                base: BuildscriptGenrule { name, .. },
                ..
            }) => name,
            Rule::BuildscriptGenruleFilter(BuildscriptGenruleFilter {
                base: BuildscriptGenrule { name, .. },
                ..
            }) => name,
            Rule::CxxLibrary(CxxLibrary {
                common: Common { name, .. },
                ..
            }) => name,
            Rule::PrebuiltCxxLibrary(PrebuiltCxxLibrary {
                common: Common { name, .. },
                ..
            }) => name,
        }
        .as_str()
    }

    pub fn is_public(&self) -> bool {
        match self {
            Rule::Alias(Alias { public, .. }) => *public,
            Rule::Binary(RustBinary {
                common:
                    RustCommon {
                        common: Common { public, .. },
                        ..
                    },
                ..
            }) => *public,
            Rule::Library(RustLibrary {
                common:
                    RustCommon {
                        common: Common { public, .. },
                        ..
                    },
                ..
            }) => *public,
            Rule::BuildscriptGenruleSrcs(_) | Rule::BuildscriptGenruleFilter(_) => false,
            Rule::CxxLibrary(CxxLibrary {
                common: Common { public, .. },
                ..
            }) => *public,
            Rule::PrebuiltCxxLibrary(PrebuiltCxxLibrary {
                common: Common { public, .. },
                ..
            }) => *public,
        }
    }

    pub fn render(&self, config: &BuckConfig, out: &mut impl Write) -> Result<(), Error> {
        match self {
            Rule::Alias(alias) => {
                out.write_all(serde_starlark::function_call(&config.alias, &alias)?.as_bytes())?;
            }
            Rule::Binary(bin) => {
                out.write_all(
                    serde_starlark::function_call(&config.rust_binary, &bin)?.as_bytes(),
                )?;
            }
            Rule::Library(lib) => {
                out.write_all(
                    serde_starlark::function_call(&config.rust_library, &lib)?.as_bytes(),
                )?;
            }
            Rule::BuildscriptGenruleFilter(lib) => {
                out.write_all(
                    serde_starlark::function_call(&config.buildscript_genrule_args, &lib)?
                        .as_bytes(),
                )?;
            }
            Rule::BuildscriptGenruleSrcs(lib) => {
                out.write_all(
                    serde_starlark::function_call(&config.buildscript_genrule_srcs, &lib)?
                        .as_bytes(),
                )?;
            }
            Rule::CxxLibrary(lib) => {
                out.write_all(
                    serde_starlark::function_call(&config.cxx_library, &lib)?.as_bytes(),
                )?;
            }
            Rule::PrebuiltCxxLibrary(lib) => {
                out.write_all(
                    serde_starlark::function_call(&config.prebuilt_cxx_library, &lib)?.as_bytes(),
                )?;
            }
        };
        out.write_all(b"\n\n")
    }
}

pub fn write_buckfile<'a>(
    config: &BuckConfig,
    rules: impl Iterator<Item = &'a Rule>,
    out: &mut impl Write,
) -> Result<(), Error> {
    out.write_all(config.generated_file_header.as_bytes())?;
    if !config.generated_file_header.is_empty() {
        out.write_all(b"\n")?;
    }

    out.write_all(config.buckfile_imports.as_bytes())?;
    if !config.buckfile_imports.is_empty() {
        out.write_all(b"\n")?;
    }

    for r in rules {
        r.render(config, out)?
    }

    Ok(())
}
