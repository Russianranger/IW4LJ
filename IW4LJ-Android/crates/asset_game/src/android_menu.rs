//! Select one usable Android installation before publishing any runtime resources.
use std::path::{Path, PathBuf};

use crate::menu_catalog::{MenuCatalog, UI_MENU_ZONES, absorb_menu_zone};
use asset_transport::{GamesRoot, IwdIndex, discover::find_iw4_zone_in_install};

pub fn android_menu_roots(preferred: &Path) -> Vec<PathBuf> {
    let mut roots = vec![preferred.to_owned()];
    for name in [
        "IW4L_ANDROID_MEDIA",
        "IW4L_ANDROID_FILES",
        "IW4L_ANDROID_LEGACY_MEDIA",
    ] {
        if let Some(base) = std::env::var_os(name).filter(|path| !path.is_empty()) {
            let candidate = PathBuf::from(base).join("mw2");
            if !roots.contains(&candidate) {
                roots.push(candidate);
            }
        }
    }
    if let Some(configured) = std::env::var_os("IW4L_GAMES").filter(|path| !path.is_empty()) {
        let candidate = PathBuf::from(configured);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
    roots
}

/// The caller runs this on a worker. The catalog returned is the one the
/// menu will display, so full decoding is done exactly once for a good root.
pub fn android_startup_menu(
    roots: &[PathBuf],
    install: impl Fn(&mut MenuCatalog) -> Result<(), String>,
) -> Result<(GamesRoot, MenuCatalog, Vec<String>), String> {
    let mut attempts = Vec::new();
    for root in roots {
        diag::android_stage(&format!("Checking MW2 installation {}", root.display()));
        match read_menu(root).and_then(|(mut catalog, lines)| {
            diag::android_stage(&format!(
                "Applying frontend definitions to {}",
                root.display()
            ));
            install(&mut catalog).map_err(|error| {
                format!("Installing frontend menus: {error}\n{}", lines.join("\n"))
            })?;
            Ok((catalog, lines))
        }) {
            Ok((catalog, lines)) => {
                let games = GamesRoot(root.clone());
                #[cfg(target_os = "android")]
                asset_transport::discover::set_android_menu_root(&games)?;
                attempts.extend(lines);
                let selected = format!("Selected MW2 root {}", root.display());
                diag::android_stage(&selected);
                attempts.push(selected);
                return Ok((games, catalog, attempts));
            }
            Err(error) => {
                let rejected = format!("Cannot use {}:\n{error}", root.display());
                diag::warn!(Launch, "{rejected}");
                attempts.push(rejected);
            }
        }
    }
    Err(format!(
        "Could not open a usable MW2 multiplayer installation. Copy the complete PC multiplayer zone/ and main/ folders into {} and choose Retry.\n\n{}",
        roots.first().map_or_else(
            || "the app's mw2 folder".into(),
            |path| path.display().to_string()
        ),
        attempts.join("\n\n")
    ))
}

fn read_menu(root: &Path) -> Result<(MenuCatalog, Vec<String>), String> {
    // Check the actual IW4 anchor within this candidate before indexing archives.
    // Do not discover common_mp in saved additional game folders or sibling titles.
    let common = find_iw4_zone_in_install(root, "common_mp")?;
    let main = root.join("main");
    diag::android_stage(&format!("Indexing MW2 archives in {}", main.display()));
    if IwdIndex::open(&main)?.archive_count() == 0 {
        return Err(format!("No readable .iwd archives in {}", main.display()));
    }

    let mut catalog = MenuCatalog::default();
    let mut lines = Vec::new();
    for &zone in UI_MENU_ZONES {
        let file = if zone == "common_mp" {
            Ok(common.clone())
        } else {
            find_iw4_zone_in_install(root, zone)
        };
        let result = file.and_then(|found| {
            diag::android_stage(&format!("Decoding {}", found.path.display()));
            absorb_menu_zone(&mut catalog, root, &found)
        });
        match result {
            Ok(summary) => lines.push(summary),
            Err(error) if zone == "common_mp" => return Err(error),
            Err(error) => lines.push(format!("menu catalog gap: {zone}: {error}")),
        }
    }
    let missing = ["main", "main_text"]
        .into_iter()
        .filter(|name| catalog.get(name).is_none())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "Required menu definitions missing: {}.\n{}",
            missing.join(", "),
            lines.join("\n")
        ));
    }
    Ok((catalog, lines))
}
