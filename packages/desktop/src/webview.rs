use crate::desktop_context::{EditQueue, EventData};
use crate::protocol::{self, AssetHandlerRegistry};
use crate::{desktop_context::UserWindowEvent, Config};
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
pub use wry;
pub use wry::application as tao;
use wry::application::menu::{MenuBar, MenuItem};
use wry::application::window::Window;
use wry::http::Response;
use wry::webview::{WebContext, WebView, WebViewBuilder};

pub(crate) fn build(
    cfg: &mut Config,
    event_loop: &EventLoopWindowTarget<UserWindowEvent>,
    proxy: EventLoopProxy<UserWindowEvent>,
) -> (WebView, WebContext, AssetHandlerRegistry, EditQueue) {
    let builder = cfg.window.clone();
    let window = builder.with_visible(false).build(event_loop).unwrap();
    let file_handler = cfg.file_drop_handler.take();
    let custom_head = cfg.custom_head.clone();
    let index_file = cfg.custom_index.clone();
    let root_name = cfg.root_name.clone();

    if cfg.enable_default_menu_bar {
        builder = builder.with_menu(build_default_menu_bar());
    }

    let window = builder.with_visible(false).build(event_loop).unwrap();

    // We assume that if the icon is None in cfg, then the user just didnt set it
    if cfg.window.window.window_icon.is_none() {
        window.set_window_icon(Some(
            tao::window::Icon::from_rgba(
                include_bytes!("./assets/default_icon.bin").to_vec(),
                460,
                460,
            )
            .expect("image parse failed"),
        ));
    }

    let mut web_context = WebContext::new(cfg.data_dir.clone());
    let edit_queue = EditQueue::default();
    let headless = !cfg.window.window.visible;
    let asset_handlers = AssetHandlerRegistry::new();
    let asset_handlers_ref = asset_handlers.clone();

    let mut webview = WebViewBuilder::new(window)
        .unwrap()
        .with_transparent(cfg.window.window.transparent)
        .with_url("dioxus://index.html/")
        .unwrap()
        .with_ipc_handler(move |window: &Window, payload: String| {
            // defer the event to the main thread
            if let Ok(message) = serde_json::from_str(&payload) {
                _ = proxy.send_event(UserWindowEvent(EventData::Ipc(message), window.id()));
            }
        })
        .with_asynchronous_custom_protocol(String::from("dioxus"), move |request, responder| {
            let custom_head = custom_head.clone();
            let index_file = index_file.clone();
            let root_name = root_name.clone();
            let asset_handlers_ref = asset_handlers_ref.clone();
            tokio::spawn(async move {
                let response_res = protocol::desktop_handler(
                    request,
                    custom_head.clone(),
                    index_file.clone(),
                    &root_name,
                    &asset_handlers_ref,
                )
                .await;
                responder.respond(response);
            });
        })
        .with_file_drop_handler(move |window, evet| {
            file_handler
                .as_ref()
                .map(|handler| handler(window, evet))
                .unwrap_or_default()
        })
        .with_web_context(&mut web_context);

    #[cfg(windows)]
    {
        // Windows has a platform specific settings to disable the browser shortcut keys
        use wry::webview::WebViewBuilderExtWindows;
        webview = webview.with_browser_accelerator_keys(false);
    }

    if let Some(color) = cfg.background_color {
        webview = webview.with_background_color(color);
    }

    // These are commented out because wry is currently broken in wry
    // let mut web_context = WebContext::new(cfg.data_dir.clone());
    // .with_web_context(&mut web_context);

    for (name, handler) in cfg.protocols.drain(..) {
        webview = webview.with_custom_protocol(name, move |r| match handler(&r) {
            Ok(response) => response,
            Err(err) => {
                tracing::error!("Error: {}", err);
                Response::builder()
                    .status(500)
                    .body(err.to_string().into_bytes().into())
                    .unwrap()
            }
        })
    }

    if cfg.disable_context_menu {
        // in release mode, we don't want to show the dev tool or reload menus
        webview = webview.with_initialization_script(
            r#"
                        if (document.addEventListener) {
                        document.addEventListener('contextmenu', function(e) {
                            e.preventDefault();
                        }, false);
                        } else {
                        document.attachEvent('oncontextmenu', function() {
                            window.event.returnValue = false;
                        });
                        }
                    "#,
        )
    } else {
        // in debug, we are okay with the reload menu showing and dev tool
        webview = webview.with_devtools(true);
    }

    (webview.build().unwrap(), web_context, asset_handlers, edit_queue)
}

/// Builds a standard menu bar depending on the users platform. It may be used as a starting point
/// to further customize the menu bar and pass it to a [`WindowBuilder`](tao::window::WindowBuilder).
/// > Note: The default menu bar enables macOS shortcuts like cut/copy/paste.
/// > The menu bar differs per platform because of constraints introduced
/// > by [`MenuItem`](tao::menu::MenuItem).
pub fn build_default_menu_bar() -> MenuBar {
    let mut menu_bar = MenuBar::new();

    // since it is uncommon on windows to have an "application menu"
    // we add a "window" menu to be more consistent across platforms with the standard menu
    let mut window_menu = MenuBar::new();
    #[cfg(target_os = "macos")]
    {
        window_menu.add_native_item(MenuItem::EnterFullScreen);
        window_menu.add_native_item(MenuItem::Zoom);
        window_menu.add_native_item(MenuItem::Separator);
    }

    window_menu.add_native_item(MenuItem::Hide);

    #[cfg(target_os = "macos")]
    {
        window_menu.add_native_item(MenuItem::HideOthers);
        window_menu.add_native_item(MenuItem::ShowAll);
    }

    window_menu.add_native_item(MenuItem::Minimize);
    window_menu.add_native_item(MenuItem::CloseWindow);
    window_menu.add_native_item(MenuItem::Separator);
    window_menu.add_native_item(MenuItem::Quit);
    menu_bar.add_submenu("Window", true, window_menu);

    // since tao supports none of the below items on linux we should only add them on macos/windows
    #[cfg(not(target_os = "linux"))]
    {
        let mut edit_menu = MenuBar::new();
        #[cfg(target_os = "macos")]
        {
            edit_menu.add_native_item(MenuItem::Undo);
            edit_menu.add_native_item(MenuItem::Redo);
            edit_menu.add_native_item(MenuItem::Separator);
        }

        edit_menu.add_native_item(MenuItem::Cut);
        edit_menu.add_native_item(MenuItem::Copy);
        edit_menu.add_native_item(MenuItem::Paste);

        #[cfg(target_os = "macos")]
        {
            edit_menu.add_native_item(MenuItem::Separator);
            edit_menu.add_native_item(MenuItem::SelectAll);
        }
        menu_bar.add_submenu("Edit", true, edit_menu);
    }

    menu_bar
}
