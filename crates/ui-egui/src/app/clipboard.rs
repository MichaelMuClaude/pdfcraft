use super::*;

/// Why nothing can be copied out of the document: its security doesn't allow it (Table 22,
/// bit 5; #568).
pub(crate) fn copy_refused() -> &'static str {
    tl!("The document's security settings don't allow copying text and images")
}

pub(crate) fn print_refused() -> &'static str {
    tl!("The document's security settings don't allow printing")
}

impl PdfCraftApp {
    /// Whether text and images may be copied out of document `id`; says why not when they may
    /// not. Reading aloud and accessibility tools don't ask (assistive technology may read
    /// anything, §7.6.4.2).
    pub(crate) fn may_copy(&mut self, id: DocId) -> bool {
        if self.session.get(id).is_none_or(pdfcraft_engine::Document::allows_copying) {
            return true;
        }
        self.notify(copy_refused().to_string());
        false
    }

    /// ⌘C / ⌘X / ⌘V copy, cut and paste text or images on canvas (unless a text field has focus).
    pub(super) fn clipboard_keys(&mut self, i: usize, ctx: &egui::Context, pasted: bool) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let paste_event_text = ctx.input(|input| {
            input.events.iter().find_map(|e| match e {
                egui::Event::Paste(text) if !text.is_empty() => Some(text.clone()),
                _ => None,
            })
        });
        if paste_event_text.is_some() {
            // A text field may own the paste; the canvas cache must still observe the new contents.
            self.last_copied_image = None;
            self.last_copied_file = None;
        }
        let free = self.dialog.is_none() && !self.palette_open && !ctx.egui_wants_keyboard_input() && !self.is_preparing();
        if !free || self.views.get(i).is_some_and(|v| v.forms.focus.is_some() || v.organize) {
            return;
        }
        let copy = ctx.input(|input| input.events.iter().any(|e| matches!(e, egui::Event::Copy)))
            || ctx.input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::C)));
        let cut = ctx.input(|input| input.events.iter().any(|e| matches!(e, egui::Event::Cut)))
            || ctx.input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::X)));
        let paste = pasted
            || ctx.input(|input| input.events.iter().any(|e| matches!(e, egui::Event::Paste(_))))
            || ctx.input_mut(|input| input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::V)));

        if copy
            && !self.copy_selected_text(i, ctx)
            && let Some(sel) = self.views.get(i).and_then(|v| v.image_selection.clone())
            && let Some(id) = self.views.get(i).map(|v| v.id)
            && self.may_copy(id)
        {
            self.copy_page_image(sel.page, sel.index);
        }
        if cut
            && let Some(sel) = self.views.get(i).and_then(|v| v.image_selection.clone())
            && let Some(id) = self.views.get(i).map(|v| v.id)
            && self.may_copy(id)
        {
            self.cut_page_image(sel.page, sel.index);
        }
        if paste {
            // A release is only a fallback for image shortcuts whose press the platform swallowed.
            // Text paste already reached its owner; an empty image clipboard needs no second notice.
            if pasted && paste_event_text.is_none() && !self.can_paste_image() {
                return;
            }
            let has_override_image = matches!(self.clipboard_override, Some(crate::create_ui::Clip::Image { .. }));
            #[cfg(not(target_arch = "wasm32"))]
            let system_has_text = if !has_override_image && self.system_clipboard { crate::os_clipboard::has_text() } else { false };
            #[cfg(target_arch = "wasm32")]
            let system_has_text = false;

            if !has_override_image && (paste_event_text.is_some() || system_has_text) {
                self.last_copied_image = None;
                self.last_copied_file = None;
                return;
            }

            let page = self.views.get(i).map_or(0, |v| v.current);
            self.paste_image(page, None);
        }
    }

    /// Shared permission gate for copying selected page text from menus and keyboard events.
    /// After Select All of the document (#2639), pages whose text wasn't read for display are read
    /// off the UI thread ([`DocumentCopy`]), and the clipboard gets the text when they're done.
    pub(crate) fn copy_selected_text(&mut self, i: usize, ctx: &egui::Context) -> bool {
        // A newer copy wins: a whole-document copy still reading must not replace it later.
        self.document_copy = None;
        let Some(v) = self.views.get(i) else { return false };
        let id = v.id;
        if v.all_text && !v.pages_without_text().is_empty() {
            // Checked before any page is read: a document that refuses copying reads nothing.
            if !self.may_copy(id) {
                return false;
            }
            let Some(job) = self.views.get(i).and_then(|v| Some(DocumentCopyJob::of(v, self.session.get(id)?))) else { return false };
            self.document_copy = Some(DocumentCopy::start(id, job, ctx));
            return true;
        }
        let Some(text) = v.selected_text().filter(|text| !text.is_empty()) else { return false };
        if !self.may_copy(id) {
            return false;
        }
        ctx.copy_text(text);
        true
    }

    /// The clipboard text of a whole-document copy, once its pages are read; dropped with the
    /// document it was copied from, and by any later copy (a text field's, a comment's), which
    /// it must not replace. Run at the end of the frame, so it sees this frame's copies.
    pub(crate) fn finish_document_copy(&mut self, ctx: &egui::Context) {
        let Some(c) = &self.document_copy else { return };
        let copied = ctx.output(|o| o.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(_) | egui::OutputCommand::CopyImage(_))));
        if copied || !self.views.iter().any(|v| v.id == c.doc) {
            self.document_copy = None;
            return;
        }
        match c.rx.try_recv() {
            Ok(text) => {
                self.document_copy = None;
                if !text.is_empty() {
                    ctx.copy_text(text);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(50)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.document_copy = None,
        }
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    #[test]
    fn review_an_image_less_release_is_silent_but_explicit_paste_explains() {
        let mut app = PdfCraftApp::new();
        app.system_clipboard = false;
        app.create_blank();
        app.toast = None;
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput::default(), |ui| app.clipboard_keys(0, ui.ctx(), true)).drop_without_applying_deltas();
        assert!(app.toast.is_none());
        assert!(!app.paste_image(0, None));
        assert!(app.toast.is_some(), "an explicit menu action still explains why there is no image");
    }

    #[test]
    fn review_text_paste_release_is_silent_and_later_image_paste_still_works() {
        let mut app = PdfCraftApp::new();
        app.system_clipboard = false;
        app.create_blank();
        app.last_copied_image = Some((1, 1, vec![255, 0, 0, 255]));
        app.palette_open = true; // A text field owns the first event.
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput { events: vec![egui::Event::Paste("text".into())], ..Default::default() }, |ui| {
            app.clipboard_keys(0, ui.ctx(), false)
        })
        .drop_without_applying_deltas();
        assert!(app.last_copied_image.is_none());
        app.palette_open = false;
        app.toast = None;
        ctx.run_ui(egui::RawInput::default(), |ui| app.clipboard_keys(0, ui.ctx(), true)).drop_without_applying_deltas();
        assert!(app.toast.is_none(), "the text gesture's key release must be silent");
        app.clipboard_override = Some(crate::create_ui::Clip::Image { width: 1, height: 1, rgba: vec![0, 255, 0, 255] });
        ctx.run_ui(egui::RawInput::default(), |ui| app.clipboard_keys(0, ui.ctx(), true)).drop_without_applying_deltas();
        let id = app.active_ids().unwrap().1;
        assert_eq!(app.session.get(id).unwrap().page_images(0).len(), 1, "a later real image shortcut still works");
    }

    #[test]
    fn review_failed_keyboard_cut_keeps_the_image_selection() {
        let mut app = PdfCraftApp::new();
        app.system_clipboard = false;
        app.create_blank();
        app.views[0].image_selection = Some(crate::edit_text_ui::ImageSelection::at(0, 0));
        let ctx = egui::Context::default();
        ctx.run_ui(egui::RawInput { events: vec![egui::Event::Cut], ..Default::default() }, |ui| app.clipboard_keys(0, ui.ctx(), false))
            .drop_without_applying_deltas();
        assert!(app.views[0].image_selection.is_some());
    }
}
