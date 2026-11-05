//! Compiled-in registry of the components the Panel installs and updates.
//!
//! Why the descriptors are compiled in instead of shipped as a `components.json` data file:
//! the set is tiny, changes only when the Panel itself changes, and every descriptor is consumed
//! by Rust code that must agree with it (download source, version scheme, dependency preflight).
//! A data file would add a second source of truth that can drift from that code — and, because
//! it lives on disk next to the executable, can be edited after install. Adding a component is
//! therefore a compile-time change by design.

use crate::update_core::DisplayVersion;
use crate::AppError;

/// Managed DLL the Local Arena plugins link against (see `install_checks` `BOTHIDER_API_MANAGED`).
const BOTHIDER_API_DLL: &str =
    "addons/counterstrikesharp/shared/BotHiderApi/BotHiderApi.dll";

/// How a component's version strings are ordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionScheme {
    /// Panel-style `MAJOR.MINOR.PATCH.BUILD[-Preview.N]`; ordered numerically.
    Panel,
    /// Third-party release tag (e.g. `3.7.2`); compared for equality only.
    UpstreamTag,
}

/// Where a component's latest release comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentSource {
    /// GitHub release assets of `repository`, picked by `asset`.
    GitHubRelease {
        repository: &'static str,
        asset: AssetSelector,
    },
    /// Field of the signed `latest.json` manifest.
    SignedLatestJson { field: &'static str },
}

/// Which GitHub release asset belongs to a component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetSelector {
    /// Asset name starts with this prefix.
    Prefix(&'static str),
    /// Asset name matches exactly.
    ExactName(&'static str),
}

/// Another component that must already be installed before this one can be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// The managed `BotHiderApi.dll` shipped by the upstream bundle.
    BotHiderApi,
}

/// One installable/updatable component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentDescriptor {
    pub id: &'static str,
    /// i18n key for the user-visible name.
    pub display_key: &'static str,
    pub version_scheme: VersionScheme,
    pub source: ComponentSource,
    pub requires: &'static [Requirement],
}

pub const UPSTREAM_BUNDLE: ComponentDescriptor = ComponentDescriptor {
    id: "upstream",
    display_key: "upstream.title",
    version_scheme: VersionScheme::UpstreamTag,
    source: ComponentSource::GitHubRelease {
        repository: "ed0ard/CS2-Bot-Improver",
        asset: AssetSelector::ExactName("CS2BotImprover.zip"),
    },
    requires: &[],
};

pub const LA_PLUGIN_BUNDLE: ComponentDescriptor = ComponentDescriptor {
    id: "la_plugins",
    display_key: "laBundle.title",
    version_scheme: VersionScheme::Panel,
    source: ComponentSource::SignedLatestJson { field: "la_plugins" },
    requires: &[Requirement::BotHiderApi],
};

pub const INVENTORY_SIMULATOR: ComponentDescriptor = ComponentDescriptor {
    id: "inventory_simulator",
    display_key: "inventoryPlugin.title",
    version_scheme: VersionScheme::UpstreamTag,
    source: ComponentSource::GitHubRelease {
        repository: "ianlucas/cs2-css-inventory-simulator",
        asset: AssetSelector::Prefix("InventorySimulator-"),
    },
    requires: &[],
};

pub const ALL: [&ComponentDescriptor; 3] = [&UPSTREAM_BUNDLE, &LA_PLUGIN_BUNDLE, &INVENTORY_SIMULATOR];

pub fn by_id(id: &str) -> Option<&'static ComponentDescriptor> {
    ALL.into_iter().find(|component| component.id == id)
}

/// Whether `latest` is newer than `current` for the given scheme.
///
/// Parse failures are errors, never a silent `false`: a malformed installed version must not
/// read as "up to date".
pub fn compare_versions(
    scheme: VersionScheme,
    current: &str,
    latest: &str,
) -> std::result::Result<bool, String> {
    match scheme {
        VersionScheme::Panel => {
            let current = DisplayVersion::parse(current)?;
            let latest = DisplayVersion::parse(latest)?;
            Ok(latest > current)
        }
        // Third-party release tags carry no ordering contract: upstream re-tags can move
        // sideways or backwards (a withdrawn release re-published under an older number), so
        // treating a tag change as "newer" would downgrade installs. Only inequality is honest.
        VersionScheme::UpstreamTag => Ok(current != latest),
    }
}

/// Fail-closed preflight: every requirement must be satisfied by the target game directory.
///
/// Requirements are checked in slice order and the first missing one is reported, so the
/// message a user sees is stable for a given descriptor.
pub fn check_requirements(
    target_root: &std::path::Path,
    requirements: &[Requirement],
) -> crate::Result<()> {
    for requirement in requirements {
        match requirement {
            Requirement::BotHiderApi => {
                if !target_root.join(BOTHIDER_API_DLL).is_file() {
                    return Err(AppError::transaction(
                        "需先安装上游套件（设置 → 安装与恢复 → 上游组件），Local Arena 插件依赖其提供的 BotHiderApi",
                    ));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_root(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "cs2bi-component-registry-{}-{label}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn descriptors_describe_three_components() {
        assert_eq!(ALL.len(), 3);

        assert_eq!(UPSTREAM_BUNDLE.id, "upstream");
        assert_eq!(UPSTREAM_BUNDLE.display_key, "upstream.title");
        assert_eq!(UPSTREAM_BUNDLE.version_scheme, VersionScheme::UpstreamTag);
        assert_eq!(
            UPSTREAM_BUNDLE.source,
            ComponentSource::GitHubRelease {
                repository: "ed0ard/CS2-Bot-Improver",
                asset: AssetSelector::ExactName("CS2BotImprover.zip"),
            }
        );
        assert!(UPSTREAM_BUNDLE.requires.is_empty());

        assert_eq!(LA_PLUGIN_BUNDLE.id, "la_plugins");
        assert_eq!(LA_PLUGIN_BUNDLE.display_key, "laBundle.title");
        assert_eq!(LA_PLUGIN_BUNDLE.version_scheme, VersionScheme::Panel);
        assert_eq!(
            LA_PLUGIN_BUNDLE.source,
            ComponentSource::SignedLatestJson { field: "la_plugins" }
        );
        assert_eq!(
            LA_PLUGIN_BUNDLE.requires,
            &[Requirement::BotHiderApi][..]
        );

        assert_eq!(INVENTORY_SIMULATOR.id, "inventory_simulator");
        assert_eq!(INVENTORY_SIMULATOR.display_key, "inventoryPlugin.title");
        assert_eq!(
            INVENTORY_SIMULATOR.version_scheme,
            VersionScheme::UpstreamTag
        );
        assert_eq!(
            INVENTORY_SIMULATOR.source,
            ComponentSource::GitHubRelease {
                repository: "ianlucas/cs2-css-inventory-simulator",
                asset: AssetSelector::Prefix("InventorySimulator-"),
            }
        );
        assert!(INVENTORY_SIMULATOR.requires.is_empty());
    }

    #[test]
    fn panel_scheme_orders_numerically() {
        assert_eq!(
            compare_versions(VersionScheme::Panel, "1.4.3.3", "1.4.2.4"),
            Ok(false)
        );
        assert_eq!(
            compare_versions(VersionScheme::Panel, "1.4.3.3", "1.4.4.0"),
            Ok(true)
        );
        assert_eq!(
            compare_versions(VersionScheme::Panel, "1.4.3.3", "1.4.3.3"),
            Ok(false)
        );
        assert_eq!(
            compare_versions(VersionScheme::Panel, "1.4.3.3", "1.4.3.3-Preview.1"),
            Ok(false)
        );
    }

    #[test]
    fn panel_scheme_rejects_malformed_versions() {
        let bad = compare_versions(VersionScheme::Panel, "bad", "1.4.3.3");
        assert!(bad.is_err(), "malformed current version must fail closed");
        let bad = compare_versions(VersionScheme::Panel, "1.4.3.3", "not-a-version");
        assert!(bad.is_err(), "malformed latest version must fail closed");
    }

    #[test]
    fn upstream_tag_scheme_compares_by_inequality_only() {
        assert_eq!(
            compare_versions(VersionScheme::UpstreamTag, "3.7.0", "3.7.2"),
            Ok(true)
        );
        assert_eq!(
            compare_versions(VersionScheme::UpstreamTag, "3.7.2", "3.7.2"),
            Ok(false)
        );
        assert_eq!(
            compare_versions(VersionScheme::UpstreamTag, "v0.6.0", "v0.5.2"),
            Ok(true)
        );
        assert_eq!(
            compare_versions(VersionScheme::UpstreamTag, "nightly", "nightly"),
            Ok(false)
        );
    }

    #[test]
    fn check_requirements_fails_closed_on_missing_bothider_api() {
        let root = temp_root("missing");
        let error = check_requirements(&root, &[Requirement::BotHiderApi])
            .expect_err("missing BotHiderApi.dll must fail the preflight");
        assert_eq!(error.code, "E1401");
        assert!(
            error.detail.contains("BotHiderApi"),
            "detail must name the missing dependency: {}",
            error.detail
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn check_requirements_passes_once_bothider_api_exists() {
        let root = temp_root("present");
        let dll = root.join(BOTHIDER_API_DLL);
        std::fs::create_dir_all(dll.parent().expect("dll has a parent")).expect("create dirs");
        std::fs::write(&dll, b"stub").expect("write stub dll");
        assert!(check_requirements(&root, &[Requirement::BotHiderApi]).is_ok());
        assert!(check_requirements(&root, &[]).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn by_id_finds_known_and_rejects_unknown() {
        assert_eq!(by_id("upstream").map(|c| c.id), Some("upstream"));
        assert_eq!(by_id("la_plugins").map(|c| c.id), Some("la_plugins"));
        assert_eq!(
            by_id("inventory_simulator").map(|c| c.id),
            Some("inventory_simulator")
        );
        assert!(by_id("nope").is_none());
        assert!(by_id("").is_none());
    }
}
