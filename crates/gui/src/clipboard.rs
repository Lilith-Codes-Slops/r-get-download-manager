//! Write to the real system clipboard, reporting failures instead of silently
//! falling back to egui's in-process clipboard. Keep ownership for X11.
#[derive(Default)]
pub struct Clipboard {
    inner: Option<arboard::Clipboard>,
}
impl Clipboard {
    pub fn write(&mut self, text: String) -> Result<(), String> {
        if self.inner.is_none() {
            self.inner = Some(arboard::Clipboard::new().map_err(|error| error.to_string())?);
        }
        self.inner
            .as_mut()
            .expect("clipboard was initialized")
            .set_text(text)
            .map_err(|error| error.to_string())
    }
}
