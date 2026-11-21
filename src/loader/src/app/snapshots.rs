use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::test::TestWindowExt;
use gpui_kit::{Bounds, HeadlessAppContext};

use super::*;
use crate::ui::theme;

pub(super) type State = (&'static str, fn(&mut LoaderApp));

fn states() -> Vec<State> {
    [
        chrome::snapshots(),
        clients::snapshots(),
        store_page::snapshots(),
        sessions::snapshots(),
        queue_page::snapshots(),
        run_page::snapshots(),
        editor::snapshots(),
        development::snapshots(),
        terminal_page::snapshots(),
        accounts_page::snapshots(),
        feedback_page::snapshots(),
        settings_page::snapshots(),
        output::snapshots(),
        dialogs::snapshots(),
    ]
    .concat()
}

pub(super) fn me() -> Me {
    Me {
        username: "tester".into(),
        role: "user".into(),
        microcredits: 12_340_000,
        min_balance_minutes: 10,
        minutes_per_month: 43_200,
        api_version: API_VERSION,
    }
}

fn config() -> Config {
    Config {
        api_url: "http://127.0.0.1:9".into(),
        supabase_url: "http://127.0.0.1:9".into(),
        anon_key: String::new(),
        site_url: "https://example.invalid".into(),
        prices_url: String::new(),
    }
}

fn isolate() -> PathBuf {
    let root = std::env::temp_dir().join(format!("loader-snapshots-{}", std::process::id()));
    for dir in ["data", "config", "cache"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    unsafe {
        std::env::set_var("XDG_DATA_HOME", root.join("data"));
        std::env::set_var("XDG_CONFIG_HOME", root.join("config"));
        std::env::set_var("XDG_CACHE_HOME", root.join("cache"));
        std::env::set_var("DBUS_SESSION_BUS_ADDRESS", "disabled:");
    }
    root
}

fn window_size() -> gpui_kit::Size<Pixels> {
    let text = std::env::var("SNAPSHOT_SIZE").unwrap_or_else(|_| "900x640".into());
    let (width, height) = text.split_once('x').expect("SNAPSHOT_SIZE is WIDTHxHEIGHT");
    size(
        px(width.parse().expect("a width")),
        px(height.parse().expect("a height")),
    )
}

#[test]
#[ignore = "writes a PNG of each page state; SNAPSHOT_DIR, SNAPSHOT_ONLY and SNAPSHOT_SIZE pick where, which and how big"]
fn pages() {
    let root = isolate();
    let dir = std::env::var("SNAPSHOT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("png"));
    std::fs::create_dir_all(&dir).unwrap();
    let only: Option<Vec<String>> = std::env::var("SNAPSHOT_ONLY")
        .ok()
        .map(|list| list.split(',').map(str::to_owned).collect());
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(gpui_kit::assets::Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        theme::install(cx, theme::UiFont::default());
    });
    for (name, setup) in states() {
        if only
            .as_ref()
            .is_some_and(|only| !only.iter().any(|wanted| name.starts_with(wanted.as_str())))
        {
            continue;
        }
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(Bounds {
                            origin: Default::default(),
                            size: window_size(),
                        })),
                        focus: false,
                        show: false,
                        ..Default::default()
                    },
                    cx,
                    |_, cx| {
                        cx.new(|cx| {
                            let mut app = LoaderApp::new(
                                Storage::at(None),
                                config(),
                                update::Instance::none(),
                                Wake::default(),
                                None,
                                Chrome::new(cx),
                            );
                            app.auth = Auth::SignedIn(me());
                            setup(&mut app);
                            app
                        })
                    },
                )
            })
            .unwrap();
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update_window(handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
        }
        let image = cx.capture_screenshot(handle).expect("offscreen rendering works");
        let path = dir.join(format!("{name}.png"));
        image.save(&path).unwrap();
        println!("{}", path.display());
        cx.update_window(handle, |_, window, _| window.remove_window())
            .unwrap();
    }
}
