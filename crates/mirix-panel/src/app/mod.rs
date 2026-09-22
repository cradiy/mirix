//! GPUI candidate panel.
//!
//! The Wayland daemon embeds it on the input-method connection so the surface
//! can use `input_popup_surface`. The standalone binary uses a non-interactive
//! layer-shell surface for demos and as the non-Wayland fallback.

mod client;
mod demo;
mod effects;
mod layers;
mod layout;
mod motion;
mod quick_switch;
mod render;
mod services;
pub mod settings;
mod skin_catalog;
mod tray;
mod view;

use anyhow::Result;
use gpui::{
    App, AppContext, Bounds, WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind,
    WindowOptions, layer_shell::*, point, px, size,
};
use gpui_platform::application;

use services::{PanelEvent, Services};
use view::{CandidatePanel, empty_snapshot};

#[derive(Debug, Clone)]
pub struct PanelOptions {
    pub demo: bool,
    pub skin_path: Option<std::path::PathBuf>,
    pub socket: std::path::PathBuf,
    pub language: Option<String>,
}

pub fn run(options: PanelOptions) -> Result<()> {
    run_application(application(), options, false)
}

pub fn run_with_wayland_popup(
    options: PanelOptions,
    connection: gpui_platform::WaylandConnection,
    role: gpui_platform::WaylandSurfaceRoleFactory,
) -> Result<()> {
    let platform = gpui_platform::wayland_platform_with_external_surface_role(connection, role);
    run_application(gpui::Application::with_platform(platform), options, true)
}

fn run_application(
    application: gpui::Application,
    options: PanelOptions,
    native_popup: bool,
) -> Result<()> {
    let demo = options.demo;
    // Previews neither read nor change the user's normal panel preferences.
    let config_path = (!demo).then(crate::config::default_path).transpose()?;
    let (skin, skin_path) = crate::config::load_skin(config_path.as_deref(), options.skin_path)?;
    let animations_enabled =
        crate::config::animation::enabled(config_path.as_deref(), skin_path.as_deref())?;
    let language = match &config_path {
        Some(path) => crate::config::resolve_language(path, options.language)?,
        None => options.language.filter(|language| language != "auto"),
    };
    let directories = crate::i18n::directories(config_path.as_deref());
    let localizer = std::sync::Arc::new(mirix_i18n::Localizer::load(
        language.as_deref(),
        &directories,
    )?);
    let socket = options.socket;
    let (services, events) = Services::start((!demo).then_some(socket.clone()), true)?;
    let commands = services.commands();
    application.run(move |cx: &mut App| {
        cx.set_global(crate::i18n::UiLanguage {
            localizer,
            preference: language,
            directories,
        });
        settings::install(commands.clone(), socket, cx);
        let skin_root = skin_path.clone();
        let show_preedit = demo || !native_popup;
        let initial_width = skin.canvas.width.min(skin.layout.max_width);
        let window = cx
            .open_window(
                WindowOptions {
                    titlebar: None,
                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                        origin: point(px(24.), px(24.)),
                        size: size(px(initial_width), px(skin.canvas.height)),
                    })),
                    app_id: Some("mirix-panel".into()),
                    window_background: WindowBackgroundAppearance::Transparent,
                    kind: if native_popup {
                        WindowKind::PopUp
                    } else {
                        WindowKind::LayerShell(LayerShellOptions {
                            namespace: "mirix".into(),
                            layer: Layer::Overlay,
                            anchor: Anchor::LEFT | Anchor::BOTTOM,
                            margin: Some((px(0.), px(0.), px(48.), px(48.))),
                            keyboard_interactivity: KeyboardInteractivity::None,
                            exclusive_zone: Some(px(0.)),
                            ..Default::default()
                        })
                    },
                    ..Default::default()
                },
                |window, cx| {
                    if !demo {
                        window.set_input_region(Some(&[]));
                    }
                    cx.new(|_| {
                        let mut panel = CandidatePanel::new(
                            if demo {
                                demo::snapshot()
                            } else {
                                empty_snapshot()
                            },
                            skin.clone(),
                            skin_root.clone(),
                            services,
                            show_preedit,
                            size(px(initial_width), px(skin.canvas.height)),
                        );
                        panel.animations_enabled = animations_enabled;
                        panel
                    })
                },
            )
            .expect("open panel window");

        window
            .update(cx, |panel, window, cx| {
                panel.update_presentation(window, cx)
            })
            .expect("initialize panel visibility");
        tray::install(window, commands, skin_path.clone(), config_path.clone(), cx);
        receive_events(window, events, cx);
    });

    Ok(())
}

fn receive_events(
    window: WindowHandle<CandidatePanel>,
    events: async_channel::Receiver<PanelEvent>,
    cx: &App,
) {
    cx.spawn(async move |cx| {
        while let Ok(event) = events.recv().await {
            // Apply queued state in one GUI update instead of rendering every
            // intermediate snapshot after a busy frame.
            let batch = std::iter::once(event)
                .chain(std::iter::from_fn(|| events.try_recv().ok()).take(255))
                .collect::<Vec<_>>();
            let status = batch.iter().rev().find_map(|event| match event {
                PanelEvent::InputMethod {
                    icon,
                    name,
                    available,
                    menus,
                    switch_shortcut,
                    ..
                } => Some(Some((
                    icon.clone(),
                    name.clone(),
                    available.clone(),
                    menus.clone(),
                    switch_shortcut.clone(),
                ))),
                PanelEvent::Disconnected => Some(None),
                _ => None,
            });
            let skin = batch.iter().rev().find_map(|event| match event {
                PanelEvent::ApplySkin(id) => Some(id.clone()),
                _ => None,
            });
            let preferences = batch
                .iter()
                .filter_map(|event| match event {
                    PanelEvent::UiPreferenceChanged(preference) => Some(*preference),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let configuration_events = batch
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        PanelEvent::InputMethod { .. }
                            | PanelEvent::ConfigurationFinished { .. }
                            | PanelEvent::Catalog(_)
                            | PanelEvent::CatalogFinished { .. }
                            | PanelEvent::Disconnected
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            if window
                .update(cx, |panel, window, cx| {
                    panel.apply_events(batch, window, cx)
                })
                .is_err()
            {
                break;
            }
            // Hiding the candidate surface takes priority over native menu work.
            cx.update(|cx| {
                let mut changed = false;
                for event in &configuration_events {
                    changed |= settings::handle(event, cx);
                }
                if changed {
                    tray::refresh(cx);
                }
            });
            if let Some(status) = status {
                cx.update(|cx| match status {
                    Some((icon, name, available, menus, switch_shortcut)) => {
                        tray::input_method(icon, name, available, menus, switch_shortcut, cx)
                    }
                    None => tray::disconnected(cx),
                });
            }
            if let Some(id) = skin {
                cx.update(|cx| tray::apply_skin(&id, cx));
            }
            for preference in preferences {
                cx.update(|cx| tray::reload_preference(preference, cx));
            }
        }
    })
    .detach();
}
