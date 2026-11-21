#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod account;
mod accounts;
mod agent_runs;
mod api;
mod app;
mod breaks;
mod counters;
#[cfg(test)]
mod fake_api;
mod feedback;
#[cfg(test)]
mod flow_tests;
mod fold;
mod form;
mod highlight;
mod http;
mod icons;
mod inventory;
mod layouts;
mod libs;
mod log_view;
mod logo;
mod mcp;
mod packet_defs;
mod packets;
mod paths;
mod pipe;
mod queue;
mod script;
mod script_log;
mod script_names;
mod session;
mod storage;
mod store;
mod terminal;
mod transports;
mod ui;
mod update;
mod vars;
mod wake;
mod whitelist;
mod widgets;
mod workspace;
mod xp;

pub const APP_ID: &str = "Loader";

fn main() {
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        let _instance = update::Instance::hold();
        mcp::serve();
        return;
    }
    #[cfg(windows)]
    if update::finish_if_asked() {
        return;
    }
    let instance = update::Instance::hold();
    if instance.alone() {
        update::cleanup();
    }
    transports::start();
    let storage = storage::Storage::open();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            app::LoaderApp::open_window(cx, storage, instance).expect("the window opens");
        });
    #[cfg(not(windows))]
    update::relaunch_after_exit();
}
