//! Compiled-in registry of the components the Panel installs and updates.
//!
//! Why the descriptors are compiled in instead of shipped as a `components.json` data file:
//! the set is tiny, changes only when the Panel itself changes, and every descriptor is consumed
//! by Rust code that must agree with it (download source, dependency preflight). A data file
//! would add a second source of truth that can drift from that code — and, because it lives on
//! disk next to the executable, can be edited after install. Adding a component is therefore a
//! compile-time change by design.

use crate::AppError;

/// Managed DLL the Local Arena plugins link against (see `install_checks` `BOTHIDER_API_MANAGED`).
const BOTHIDER_API_DLL: &str =
    "addons/counterstrikesharp/shared/BotHiderApi/BotHiderApi.dll";

/// Where a component's latest release comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComponentSource {
    /// GitHub release assets of `repository`, picked by `asset`.
    GitHubRelease {
        repository: &'static str,
        asset: AssetName,
    },
    /// Field of the signed `latest.json` manifest.
    SignedLatestJson { field: &'static str },
}

/// Which GitHub release asset belongs to a component, as compile-time data.
///
/// The matching rules live exactly once, in [`crate::release_source::AssetSelector`];
/// [`AssetName::selector`] builds that runtime selector, so the descriptor cannot drift from
/// the matching logic `release_source` actually applies. A descriptor cannot hold the selector
/// directly: it is a `const`, and the selector's owned strings cannot be built at compile time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetName {
    /// Asset name starts with this prefix.
    Prefix(&'static str),
    /// Asset name matches exactly.
    Exact(&'static str),
}

impl AssetName {
    /// The runtime selector `release_source` resolves releases with.
    pub fn selector(&self) -> crate::release_source::AssetSelector {
        match self {
            AssetName::Prefix(prefix) => {
                crate::release_source::AssetSelector::Prefix((*prefix).to_string())
            }
            AssetName::Exact(name) => {
                crate::release_source::AssetSelector::ExactName((*name).to_string())
            }
        }
    }
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
    pub source: ComponentSource,
    pub requires: &'static [Requirement],
}

pub const UPSTREAM_BUNDLE: ComponentDescriptor = ComponentDescriptor {
    id: "upstream",
    display_key: "upstream.title",
    source: ComponentSource::GitHubRelease {
        repository: "ed0ard/CS2-Bot-Improver",
        asset: AssetName::Exact("CS2BotImprover.zip"),
    },
    requires: &[],
};

pub const LA_PLUGIN_BUNDLE: ComponentDescriptor = ComponentDescriptor {
    id: "la_plugins",
    display_key: "laBundle.title",
    source: ComponentSource::SignedLatestJson { field: "la_plugins" },
    requires: &[Requirement::BotHiderApi],
};

pub const INVENTORY_SIMULATOR: ComponentDescriptor = ComponentDescriptor {
    id: "inventory_simulator",
    display_key: "inventoryPlugin.title",
    source: ComponentSource::GitHubRelease {
        repository: "ianlucas/cs2-css-inventory-simulator",
        asset: AssetName::Prefix("InventorySimulator-"),
    },
    requires: &[],
};

pub const ALL: [&ComponentDescriptor; 3] = [&UPSTREAM_BUNDLE, &LA_PLUGIN_BUNDLE, &INVENTORY_SIMULATOR];

/// The registry row for `id` — the same id the update-progress events and the update manifest
/// field names carry.
pub fn by_id(id: &str) -> Option<&'static ComponentDescriptor> {
    ALL.into_iter().find(|component| component.id == id)
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
        assert_eq!(
            UPSTREAM_BUNDLE.source,
            ComponentSource::GitHubRelease {
                repository: "ed0ard/CS2-Bot-Improver",
                asset: AssetName::Exact("CS2BotImprover.zip"),
            }
        );
        assert!(UPSTREAM_BUNDLE.requires.is_empty());

        assert_eq!(LA_PLUGIN_BUNDLE.id, "la_plugins");
        assert_eq!(LA_PLUGIN_BUNDLE.display_key, "laBundle.title");
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
            INVENTORY_SIMULATOR.source,
            ComponentSource::GitHubRelease {
                repository: "ianlucas/cs2-css-inventory-simulator",
                asset: AssetName::Prefix("InventorySimulator-"),
            }
        );
        assert!(INVENTORY_SIMULATOR.requires.is_empty());
    }

    #[test]
    fn own_asset_names_build_release_source_selectors() {
        let ComponentSource::GitHubRelease { asset, .. } = INVENTORY_SIMULATOR.source else {
            panic!("InventorySimulator installs from a GitHub release");
        };
        let selector = asset.selector();
        assert!(selector.matches("InventorySimulator-v3.7.0.zip"));
        assert!(
            !selector.matches("CS2BotImprover.zip"),
            "the selector must be the component's own, not the upstream one"
        );

        let ComponentSource::GitHubRelease { asset, .. } = UPSTREAM_BUNDLE.source else {
            panic!("the upstream bundle installs from a GitHub release");
        };
        let selector = asset.selector();
        assert!(selector.matches("CS2BotImprover.zip"));
        assert!(!selector.matches("InventorySimulator-v3.7.0.zip"));
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
