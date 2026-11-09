use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};

use bevy::render::error_handler::{RenderErrorHandler, RenderErrorPolicy};

use super::*;

#[derive(Resource)]
struct PendingMenu(Mutex<Receiver<Result<PreparedMenu, String>>>);

pub(super) fn run(games: asset_transport::GamesRoot, artifacts: PathBuf, cheats: sim::HostCheats) {
    start_perf(None, "menu");
    let config = LaunchConfig {
        role: Role::Listen,
        zone: String::new(),
        games_root: PathBuf::new(),
        artifacts,
    };
    diag::android_stage("Initializing window, renderer and engine plugins");
    // NetPlugin chooses its schedules when built. Keep the browser dormant until
    // the worker publishes the actual installed namespace inventory.
    let master = menu_master_intent(net::content_inventory(true, false, false, false));
    let mut app = menu_app(&config, cheats, master, None);
    let artifacts = config.artifacts.clone();
    app.insert_resource(net::MasterLaunchIntent::disabled())
        .insert_resource(AppScreen::Loading)
        .insert_resource(RenderErrorHandler(|error, world, _| {
            diag::android_failure(&format!("Renderer stopped: {error:?}"));
            world.write_message(AppExit::error());
            RenderErrorPolicy::StopRendering
        }))
        .add_systems(
            First,
            |mut events: MessageReader<bevy::window::AppLifecycle>| {
                for event in events.read() {
                    diag::android_event(&format!("winit lifecycle {event:?}"));
                }
            },
        )
        .add_systems(Startup, move |mut commands: Commands| {
            diag::android_stage("First winit update; starting menu preparation");
            let preferred = games.0.clone();
            let artifacts = artifacts.clone();
            let (send, receive) = mpsc::channel();
            match std::thread::Builder::new()
                .name("IW4 menu preparation".into())
                .stack_size(8 * 1024 * 1024)
                .spawn(move || {
                    let saved = console::stored_game_folders(&artifacts);
                    asset_transport::set_game_folders(saved.clone());
                    let mut roots = asset_game::android_menu_roots(&preferred);
                    for folder in saved {
                        if !roots.contains(&folder) {
                            roots.push(folder);
                        }
                    }
                    let result = prepare(&roots);
                    let _ = send.send(result);
                }) {
                Ok(_) => commands.insert_resource(PendingMenu(Mutex::new(receive))),
                Err(error) => {
                    diag::android_failure(&format!("Could not start menu worker: {error}"))
                }
            }
        })
        .add_systems(PreUpdate, poll_menu.run_if(resource_exists::<PendingMenu>));
    diag::android_stage("Entering winit event loop");
    run_menu_app(app, &config.artifacts);
}

fn prepare(roots: &[PathBuf]) -> Result<PreparedMenu, String> {
    let (games, menus, report) =
        asset_game::android_startup_menu(roots, ui::install_frontend_menus)?;
    for line in report {
        diag::info!(Launch, "{line}");
    }
    let shell = assets::load_pool().spawn(assets::load_shell_common(games.clone()));
    Ok(prepare_menu(games.clone(), games, menus, shell))
}

fn poll_menu(world: &mut World) {
    let result = world
        .resource::<PendingMenu>()
        .0
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .try_recv();
    let result = match result {
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => {
            Err("Menu worker stopped before returning a result. See its panic report.".into())
        }
        Ok(result) => result,
    };
    world.remove_resource::<PendingMenu>();
    match result {
        Ok(menu) => {
            diag::android_stage(&format!(
                "Installing menu from {}; preparing common assets",
                menu.games.0.display()
            ));
            install_menu(world, menu);
        }
        Err(error) => diag::android_failure(&format!(
            "Menu preparation failed. Restore the listed files from your MW2 Multiplayer installation and choose Retry.\n{error}"
        )),
    }
}
