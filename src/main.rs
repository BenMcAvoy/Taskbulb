#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use reqwest::Client;
use tao::{
    event::Event,
    event_loop::{ControlFlow, EventLoopBuilder},
};

mod light;
mod mouse;
use light::LightController;
use tray_icon::{
    Icon, MouseButton, TrayIconBuilder, TrayIconEvent,
    menu::{AboutMetadata, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

enum UserEvent {
    TrayIconEvent(tray_icon::TrayIconEvent),
    MenuEvent(tray_icon::menu::MenuEvent),
    LightState {
        is_on: bool,
        hue: f64,
        saturation: f64,
    },
    #[cfg(target_os = "windows")]
    GlobalWheel {
        delta: i16,
        ctrl: bool,
        alt: bool,
    },
    #[cfg(target_os = "windows")]
    Tooltip {
        h: f64,
        s: f64,
        v: i64,
    },
}

fn set_tooltip(tray_icon: &mut tray_icon::TrayIcon, h: f64, s: f64, v: i64) {
    let _ = tray_icon.set_tooltip(Some(format!(
        "H: {:.0}, S: {:.0}%, V: {:.0}%",
        h,
        s,
        v as f64 * 100.0 / 255.0
    )));
}

#[tokio::main]
async fn main() {
    let light = LightController::new(Client::new());

    let initial_state = light.state().await;
    let is_light_on = initial_state.as_ref().map(|s| s.is_on).unwrap_or(false);

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();

    #[cfg(target_os = "windows")]
    mouse::install(event_loop.create_proxy());

    // set a tray event handler that forwards the event and wakes up the event loop
    let proxy = event_loop.create_proxy();
    TrayIconEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::TrayIconEvent(event));
    }));

    // set a menu event handler that forwards the event and wakes up the event loop
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(UserEvent::MenuEvent(event));
    }));

    let tray_menu = Menu::new();

    let quit_i = MenuItem::new("Quit", true, None);
    let _ = tray_menu.append_items(&[
        &PredefinedMenuItem::about(
            None,
            Some(AboutMetadata {
                name: Some("Taskbulb".to_string()),
                copyright: Some("Copyright Ben McAvoy, MIT License".to_string()),
                website: Some("https://github.com/benmcavoy/taskbulb".to_string()),
                ..Default::default()
            }),
        ),
        &PredefinedMenuItem::separator(),
        &quit_i,
    ]);

    let mut tray_icon = None;
    let click_proxy = event_loop.create_proxy();

    #[cfg(target_os = "windows")]
    let mut light_updates = light.subscribe();
    #[cfg(target_os = "windows")]
    {
        let proxy = click_proxy.clone();
        tokio::spawn(async move {
            while let Ok(state) = light_updates.recv().await {
                let _ = proxy.send_event(UserEvent::LightState {
                    is_on: state.is_on,
                    hue: state.hue,
                    saturation: state.saturation,
                });
                let _ = proxy.send_event(UserEvent::Tooltip {
                    h: state.hue,
                    s: state.saturation,
                    v: state.brightness,
                });
            }
        });
    }

    let _menu_channel = MenuEvent::receiver();
    let _tray_channel = TrayIconEvent::receiver();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::NewEvents(tao::event::StartCause::Init) => {
                //let icon = load_icon(std::path::Path::new(path));

                // We create the icon once the event loop is actually running
                // to prevent issues like https://github.com/tauri-apps/tray-icon/issues/90
                tray_icon = Some(
                    TrayIconBuilder::new()
                        .with_menu(Box::new(tray_menu.clone()))
                        .with_menu_on_left_click(false)
                        .with_menu_on_right_click(true)
                        .with_icon(light_icon(
                            is_light_on,
                            initial_state.as_ref().map(|s| s.hue).unwrap_or(0.0),
                            initial_state.as_ref().map(|s| s.saturation).unwrap_or(0.0),
                        ))
                        .with_tooltip(format!(
                            "H: {:.0}, S: {:.0}%, V: {:.0}%",
                            initial_state.as_ref().map(|s| s.hue).unwrap_or(0.0),
                            initial_state.as_ref().map(|s| s.saturation).unwrap_or(0.0),
                            initial_state
                                .as_ref()
                                .map(|s| s.brightness as f64 * 100.0 / 255.0)
                                .unwrap_or(0.0)
                        ))
                        .build()
                        .unwrap(),
                );

                // We have to request a redraw here to have the icon actually show up.
                // Tao only exposes a redraw method on the Window so we use core-foundation directly.
                #[cfg(target_os = "macos")]
                unsafe {
                    use objc2_core_foundation::CFRunLoop;

                    let rl = CFRunLoop::main().unwrap();
                    CFRunLoop::wake_up(&rl);
                }
            }

            Event::UserEvent(UserEvent::TrayIconEvent(event)) => {
                #[cfg(target_os = "windows")]
                mouse::update_tray_bounds(&event);

                //if event == TrayIconEvent::Click { id: (), position: (), rect: (), button: (), button_state: () }
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    ..
                } = event
                {
                    let light = light.clone();
                    let proxy = click_proxy.clone();
                    tokio::spawn(async move {
                        if light.toggle().await.is_ok()
                            && let Ok(state) = light.state().await
                        {
                            let _ = proxy.send_event(UserEvent::LightState {
                                is_on: state.is_on,
                                hue: state.hue,
                                saturation: state.saturation,
                            });
                            let _ = proxy.send_event(UserEvent::Tooltip {
                                h: state.hue,
                                s: state.saturation,
                                v: state.brightness,
                            });
                        }
                    });
                } else if let TrayIconEvent::Click {
                    button: MouseButton::Middle,
                    ..
                } = event
                {
                    light.queue_color_reset();
                }
            }

            Event::UserEvent(UserEvent::LightState {
                is_on,
                hue,
                saturation,
            }) => {
                if let Some(tray_icon) = tray_icon.as_ref() {
                    let _ = tray_icon.set_icon(Some(light_icon(is_on, hue, saturation)));
                }
            }

            #[cfg(target_os = "windows")]
            Event::UserEvent(UserEvent::Tooltip { h, s, v }) => {
                if let Some(tray_icon) = tray_icon.as_mut() {
                    set_tooltip(tray_icon, h, s, v);
                }
            }

            #[cfg(target_os = "windows")]
            Event::UserEvent(UserEvent::GlobalWheel { delta, ctrl, alt }) => {
                let step = (delta / 30) as f64;
                if ctrl {
                    light.queue_hue_step(step * 3.0);
                } else if alt {
                    light.queue_saturation_step(step);
                } else {
                    light.queue_brightness_step(step as i16);
                }
            }

            Event::UserEvent(UserEvent::MenuEvent(event)) if event.id == quit_i.id() => {
                #[cfg(target_os = "windows")]
                mouse::uninstall();
                tray_icon.take();
                *control_flow = ControlFlow::Exit;
            }

            _ => {}
        }
    });
}

fn light_icon(is_on: bool, hue: f64, saturation: f64) -> Icon {
    #[cfg(target_os = "windows")]
    {
        use fontdue::{Font, FontSettings};

        const SIZE: usize = 32;
        const FONT_SIZE: f32 = 25.0;
        const GLYPH: char = '\u{EA80}'; // Segoe MDL2 Assets: Lightbulb
        const FONT_PATH: &str = r"C:\Windows\Fonts\segmdl2.ttf";

        let bytes = std::fs::read(FONT_PATH).expect("Segoe MDL2 Assets font is required");
        let font = Font::from_bytes(bytes, FontSettings::default())
            .expect("invalid Segoe MDL2 Assets font");

        let (metrics, bitmap) = font.rasterize(GLYPH, FONT_SIZE);

        let color = hsv_to_rgb(hue, saturation);
        let mut rgba = vec![0; SIZE * SIZE * 4];

        // Center the glyph
        let (mut min_x, mut min_y) = (metrics.width, metrics.height);
        let (mut max_x, mut max_y) = (0, 0);
        let mut has_pixels = false;
        for y in 0..metrics.height {
            for x in 0..metrics.width {
                if bitmap[y * metrics.width + x] != 0 {
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x);
                    max_y = max_y.max(y);
                    has_pixels = true;
                }
            }
        }

        let (left, top) = if has_pixels {
            (
                (SIZE as isize - (max_x - min_x + 1) as isize) / 2 - min_x as isize,
                (SIZE as isize - (max_y - min_y + 1) as isize) / 2 - min_y as isize,
            )
        } else {
            (0, 0)
        };

        for y in 0..metrics.height {
            for x in 0..metrics.width {
                let px = left + x as isize;
                let py = top + y as isize;
                if px >= 0 && px < SIZE as isize && py >= 0 && py < SIZE as isize {
                    let offset = (py as usize * SIZE + px as usize) * 4;
                    rgba[offset..offset + 3].copy_from_slice(&color[..3]);
                    rgba[offset + 3] = bitmap[y * metrics.width + x];
                }
            }
        }

        if !is_on {
            // Diagonal line to indicate the light is off

            for i in 0..22usize {
                let x = 5 + i;
                let y = 26 - i;
                for thickness in 0..2usize {
                    let px = x + thickness;
                    if px < SIZE && y < SIZE {
                        let offset = (y * SIZE + px) * 4;
                        rgba[offset..offset + 4].copy_from_slice(&color);
                    }
                }
            }
        }

        Icon::from_rgba(rgba, SIZE as u32, SIZE as u32).expect("invalid tray icon bitmap")
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (is_on, hue, saturation);
        Icon::from_rgba(vec![0; 32 * 32 * 4], 32, 32).expect("invalid tray icon bitmap")
    }
}

fn hsv_to_rgb(hue: f64, saturation: f64) -> [u8; 4] {
    let h = hue.clamp(0.0, 255.0) * 6.0 / 255.0;
    let s = saturation.clamp(0.0, 100.0) / 100.0;
    let c = s;
    let m = 1.0 - c;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    [
        ((r + m) * 255.0).round() as u8,
        ((g + m) * 255.0).round() as u8,
        ((b + m) * 255.0).round() as u8,
        255,
    ]
}
