//! The system clipboard, written by the render server itself (arboard, as the UI crate uses it:
//! `crates/ui-egui/src/agent_chat/paste.rs:36`): no browser clipboard permission, and the same
//! in WKWebView and WebView2. Kept open for the process's life: on X11 the owner serves the text.

use std::borrow::Cow;

#[derive(Default)]
pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
}

impl Clipboard {
    /// Put `text` on the clipboard; false where there is none (a Linux host with no display).
    pub fn set_text(&mut self, text: &str) -> bool {
        self.with(|c| c.set_text(text.to_string()))
    }

    pub fn set_image(&mut self, image: &egui::ColorImage) -> bool {
        let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
        let [width, height] = image.size;
        self.with(|c| c.set_image(arboard::ImageData { width, height, bytes: Cow::Owned(bytes) }))
    }

    fn with(&mut self, f: impl FnOnce(&mut arboard::Clipboard) -> Result<(), arboard::Error>) -> bool {
        if self.inner.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.inner = Some(c),
                Err(e) => {
                    log::warn!("clipboard: none on this system ({e})");
                    return false;
                }
            }
        }
        let Some(c) = self.inner.as_mut() else { return false };
        match f(c) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("clipboard: {e}");
                false
            }
        }
    }
}
