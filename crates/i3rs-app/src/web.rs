//! Web entry point for the i3rs egui app.

use std::cell::RefCell;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;
use web_sys::{Document, Element, HtmlCanvasElement};

thread_local! {
    static WEB_HANDLE: RefCell<Option<WebHandle>> = const { RefCell::new(None) };
}

/// JavaScript handle for booting the app on a canvas.
#[derive(Clone)]
#[wasm_bindgen]
pub struct WebHandle {
    runner: eframe::WebRunner,
}

#[wasm_bindgen]
impl WebHandle {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        eframe::WebLogger::init(log::LevelFilter::Debug).ok();
        Self {
            runner: eframe::WebRunner::new(),
        }
    }

    #[wasm_bindgen]
    pub async fn start(&self, canvas: HtmlCanvasElement) -> Result<(), wasm_bindgen::JsValue> {
        self.runner
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|cc| Ok(Box::new(crate::App::new(cc)))),
            )
            .await
    }

    #[wasm_bindgen]
    pub fn destroy(&self) {
        self.runner.destroy();
    }

    #[wasm_bindgen]
    pub fn load_session(
        &self,
        file_name: String,
        ld_bytes: Vec<u8>,
        ldx_xml: Option<String>,
    ) -> Result<(), wasm_bindgen::JsValue> {
        let ldx = match ldx_xml {
            Some(xml) => Some(
                i3rs_core::LdxFile::parse(&xml)
                    .map_err(|err| wasm_bindgen::JsValue::from_str(&err))?,
            ),
            None => None,
        };

        let mut app = self
            .runner
            .app_mut::<crate::App>()
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("app has not started yet"))?;

        app.open_bytes(file_name, ld_bytes, ldx)
            .map_err(|err| wasm_bindgen::JsValue::from_str(&err))
    }

    #[wasm_bindgen]
    pub fn has_panicked(&self) -> bool {
        self.runner.has_panicked()
    }

    #[wasm_bindgen]
    pub fn session_summary_json(&self) -> Option<String> {
        let app = self.runner.app_mut::<crate::App>()?;
        let summary = app.loaded_session_summary()?;
        serde_json::to_string(&summary).ok()
    }

    #[wasm_bindgen]
    pub fn panic_message(&self) -> Option<String> {
        self.runner.panic_summary().map(|summary| summary.message())
    }
}

fn focus_element(element: &Element) {
    if let Some(element) = element.dyn_ref::<web_sys::HtmlElement>() {
        element.focus().ok();
    }
}

fn blur_element(element: &Element) {
    if let Some(element) = element.dyn_ref::<web_sys::HtmlElement>() {
        element.blur().ok();
    }
}

fn is_body_element(document: &Document, element: &Element) -> bool {
    document
        .body()
        .is_some_and(|body| element.is_same_node(Some(body.as_ref())))
}

fn restore_startup_focus(document: &Document, previous_focus: Option<Element>) {
    match previous_focus {
        Some(element) if !is_body_element(document, &element) => focus_element(&element),
        _ => {
            if let Some(active_element) = document.active_element() {
                blur_element(&active_element);
            }
        }
    }
}

#[wasm_bindgen(start)]
pub fn bootstrap() -> Result<(), wasm_bindgen::JsValue> {
    let window =
        web_sys::window().ok_or_else(|| wasm_bindgen::JsValue::from_str("window not available"))?;
    let document = window
        .document()
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("document not available"))?;
    let canvas = document
        .get_element_by_id("i3rs-canvas")
        .ok_or_else(|| wasm_bindgen::JsValue::from_str("missing #i3rs-canvas element"))?
        .dyn_into::<web_sys::HtmlCanvasElement>()?;

    let handle = WebHandle::new();
    WEB_HANDLE.with(|slot| {
        *slot.borrow_mut() = Some(handle.clone());
    });

    wasm_bindgen_futures::spawn_local(async move {
        let previous_focus = document.active_element();

        if let Err(err) = handle.start(canvas).await {
            log::error!("failed to start web app: {err:?}");
            return;
        }

        restore_startup_focus(&document, previous_focus);

        if let Some(window) = web_sys::window() {
            let handle_for_js = handle.clone();
            let load = Closure::<dyn FnMut(String, js_sys::Uint8Array, JsValue)>::new(
                move |file_name: String, ld_bytes: js_sys::Uint8Array, ldx_value: JsValue| {
                    let ldx_xml = if ldx_value.is_undefined() || ldx_value.is_null() {
                        None
                    } else {
                        ldx_value.as_string()
                    };

                    if let Err(err) =
                        handle_for_js.load_session(file_name, ld_bytes.to_vec(), ldx_xml)
                    {
                        wasm_bindgen::throw_val(err);
                    }
                },
            );

            let _ = js_sys::Reflect::set(
                window.as_ref(),
                &JsValue::from_str("i3rsLoadSession"),
                load.as_ref().unchecked_ref(),
            );
            load.forget();

            let info_handle = handle.clone();
            let session_info = Closure::<dyn FnMut() -> JsValue>::new(move || {
                info_handle
                    .session_summary_json()
                    .map(|json| JsValue::from_str(&json))
                    .unwrap_or(JsValue::NULL)
            });
            let _ = js_sys::Reflect::set(
                window.as_ref(),
                &JsValue::from_str("i3rsSessionSummary"),
                session_info.as_ref().unchecked_ref(),
            );
            session_info.forget();
        }
    });

    Ok(())
}
