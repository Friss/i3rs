//! Small platform-specific helpers used by the app shell.

use std::path::PathBuf;

#[cfg(target_arch = "wasm32")]
use std::sync::mpsc::{Receiver, Sender, channel};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc::{
    Receiver as NativeReceiver, Sender as NativeSender, channel as native_channel,
};

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub enum WebLoadEvent {
    SessionData {
        file_name: String,
        ld_bytes: Vec<u8>,
        ldx_xml: Option<String>,
    },
    Error(String),
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub enum NativePickEvent {
    SessionPath(PathBuf),
}

#[cfg(not(target_arch = "wasm32"))]
fn native_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pick_overlay_file() -> Option<PathBuf> {
    native_dialog().add_filter("MoTeC Log", &["ld"]).pick_file()
}

#[cfg(target_arch = "wasm32")]
pub fn pick_overlay_file() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_project_file(suggested_name: &str) -> Option<PathBuf> {
    native_dialog()
        .add_filter("i3rs Project", &["i3rsproj", "json"])
        .set_file_name(suggested_name)
        .save_file()
}

#[cfg(target_arch = "wasm32")]
pub fn save_project_file(_suggested_name: &str) -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pick_project_file() -> Option<PathBuf> {
    native_dialog()
        .add_filter("i3rs Project", &["i3rsproj", "json"])
        .pick_file()
}

#[cfg(target_arch = "wasm32")]
pub fn pick_project_file() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_workspace_file() -> Option<PathBuf> {
    native_dialog()
        .add_filter("Workspace", &["json"])
        .set_file_name("workspace.json")
        .save_file()
}

#[cfg(target_arch = "wasm32")]
pub fn save_workspace_file() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pick_workspace_file() -> Option<PathBuf> {
    native_dialog()
        .add_filter("Workspace", &["json"])
        .pick_file()
}

#[cfg(target_arch = "wasm32")]
pub fn pick_workspace_file() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_csv_file() -> Option<PathBuf> {
    native_dialog()
        .add_filter("CSV", &["csv"])
        .set_file_name("export.csv")
        .save_file()
}

#[cfg(target_arch = "wasm32")]
pub fn save_csv_file() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_math_channels_file() -> Option<PathBuf> {
    native_dialog()
        .add_filter("Math Channels", &["json"])
        .set_file_name("math_channels.json")
        .save_file()
}

#[cfg(target_arch = "wasm32")]
pub fn save_math_channels_file() -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn pick_math_channels_file() -> Option<PathBuf> {
    native_dialog()
        .add_filter("Math Channels", &["json"])
        .pick_file()
}

#[cfg(target_arch = "wasm32")]
pub fn pick_math_channels_file() -> Option<PathBuf> {
    None
}

#[cfg(target_arch = "wasm32")]
pub fn web_load_channel() -> (Sender<WebLoadEvent>, Receiver<WebLoadEvent>) {
    channel()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn native_pick_channel() -> (
    NativeSender<NativePickEvent>,
    NativeReceiver<NativePickEvent>,
) {
    native_channel()
}

#[cfg(target_arch = "wasm32")]
pub fn begin_pick_session(tx: Sender<WebLoadEvent>, ctx: egui::Context) {
    wasm_bindgen_futures::spawn_local(async move {
        let Some(ld_handle) = rfd::AsyncFileDialog::new()
            .set_title("Select a MoTeC .ld file")
            .add_filter("MoTeC Log", &["ld"])
            .pick_file()
            .await
        else {
            return;
        };

        let file_name = ld_handle.file_name();
        let ld_bytes = ld_handle.read().await;

        let ldx_xml = match rfd::AsyncFileDialog::new()
            .set_title("Optional: select a matching .ldx file (cancel to skip)")
            .add_filter("MoTeC Sidecar", &["ldx"])
            .pick_file()
            .await
        {
            Some(ldx_handle) => match String::from_utf8(ldx_handle.read().await) {
                Ok(xml) => Some(xml),
                Err(err) => {
                    let _ = tx.send(WebLoadEvent::Error(format!(
                        "Failed to decode .ldx file as UTF-8 text: {err}"
                    )));
                    ctx.request_repaint();
                    return;
                }
            },
            None => None,
        };

        let _ = tx.send(WebLoadEvent::SessionData {
            file_name,
            ld_bytes,
            ldx_xml,
        });
        ctx.request_repaint();
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn begin_pick_session(tx: NativeSender<NativePickEvent>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let picked = pollster::block_on(
            rfd::AsyncFileDialog::new()
                .set_title("Select a MoTeC .ld file")
                .add_filter("MoTeC Log", &["ld"])
                .pick_file(),
        );

        if let Some(handle) = picked {
            let _ = tx.send(NativePickEvent::SessionPath(handle.path().to_path_buf()));
            ctx.request_repaint();
        }
    });
}
