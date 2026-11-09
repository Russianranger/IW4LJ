//! Select one usable Android installation before publishing any runtime resources.
use std::path::{Path, PathBuf};

use crate::menu_catalog::{MenuCatalog, UI_MENU_ZONES, ZoneUiImage, absorb_menu_zone};
use asset_core::{AssetNamespace, AssetRef};
use asset_transport::{GamesRoot, IwdIndex, discover::find_iw4_zone_in_install};

pub fn android_menu_roots(preferred: &Path) -> Vec<PathBuf> {
    if let Ok(encoded) = std::env::var("IW4L_ANDROID_GAME_ROOTS") {
        match serde_json::from_str::<Vec<PathBuf>>(&encoded) {
            Ok(roots) if !roots.is_empty() => return roots,
            Ok(_) => diag::warn!(Launch, "Android supplied no MW2 search paths"),
            Err(error) => diag::warn!(Launch, "Invalid Android MW2 search paths: {error}"),
        }
    }
    let mut roots = vec![preferred.to_owned()];
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
    let archives = IwdIndex::open(&main)?;
    if archives.archive_count() == 0 {
        return Err(format!("No readable .iwd archives in {}", main.display()));
    }

    // Resolve IWD stubs against this candidate's index. A donor folder in
    // settings must not satisfy another installation's font check.
    let archive_image =
        |name: &str| asset_material::decode_ui_image_from_index(archives.clone(), name);
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
            absorb_menu_zone(&mut catalog, &found, &archive_image)
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
    diag::android_stage(&format!("Checking menu label font in {}", root.display()));
    prepare_label_font(&mut catalog, &main, &archive_image)
        .map_err(|error| format!("{error}\n{}", lines.join("\n")))?;
    Ok((catalog, lines))
}

fn prepare_label_font(
    catalog: &mut MenuCatalog,
    main: &Path,
    archive_image: impl Fn(&str) -> Result<Option<(u32, u32, Vec<u8>)>, String>,
) -> Result<(), String> {
    // The production dashboard rows use font enum 3 (fonts/smallfont).
    let font = catalog.font("fonts/smallfont").filter(|font| {
        font.pixel_height > 0
            && !font.material.is_empty()
            && b"CREATE".iter().all(|letter| {
                font.glyph(u32::from(*letter))
                    .is_some_and(|g| g.pixel_width > 0 && g.pixel_height > 0)
            })
    }).ok_or("MW2 menu font fonts/smallfont has no usable glyphs or material; restore the multiplayer FastFiles.")?;
    let material = AssetRef::bare_name(&font.material).to_ascii_lowercase();
    if let Some(state) = catalog.material_state_bits.get(&material) {
        let words = state.agreed().ok_or_else(|| {
            format!("fonts/smallfont material {material}: unusable render state {state:?}")
        })?;
        let compiled = asset_material::compile_material_state(AssetNamespace::Iw4, words);
        if let Some(fields) = compiled.unsupported_host_fields() {
            return Err(format!(
                "fonts/smallfont material {material}: unsupported render state {fields:?}"
            ));
        }
    }
    if catalog.zone_image(&material).is_some() {
        return Ok(());
    }
    let image = catalog
        .material_image(&material)
        .unwrap_or(&material)
        .to_owned();
    // Font and material definitions can be in different menu FastFiles. Reuse
    // a decoded binding before consulting the candidate's archives.
    let atlas = if let Some(atlas) = catalog.zone_image(&image) {
        let mut atlas = atlas.clone();
        atlas.material = material.clone();
        atlas
    } else {
        let context = format!(
            "fonts/smallfont material {material}: images/{image}.iwi in {}",
            main.display()
        );
        let (width, height, rgba) = archive_image(&image)
            .map_err(|error| format!("{context}: {error}"))?
            .ok_or_else(|| {
                let cause = catalog
                    .zone_image_errors
                    .get(&material)
                    .map(String::as_str)
                    .unwrap_or("no decoded atlas in the menu FastFiles");
                format!("{context}: image missing from IWD archives; {cause}")
            })?;
        ZoneUiImage {
            material: material.clone(),
            image,
            width,
            height,
            rgba: rgba.into(),
            source: "iwd",
        }
    };
    catalog.zone_images.insert(material, atlas);
    Ok(())
}
