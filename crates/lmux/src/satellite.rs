//! GTK widget hosting a single nested-Wayland toplevel (ADR-0018 + Task #10).
//!
//! A `SatelliteWidget` is the cockpit-side representation of one
//! `xdg_toplevel` surface tracked by [`lmux_wayland_host`]. Its rendering
//! path is pull-only:
//!
//! * [`lmux_wayland_host::HostEvent::FrameReady`] arrives on the event
//!   channel (see `bus::run_host_event_dispatcher`) carrying RGB8 pixels.
//! * The dispatcher looks up the matching `SatelliteWidget` by
//!   [`lmux_wayland_host::SurfaceId`] and calls [`SatelliteWidget::push_frame`].
//! * The widget feeds the pixels into a `gdk::MemoryTexture` and swaps
//!   it into the child `gtk4::Picture`, which repaints on the next frame.
//!
//! Resize flow: the widget listens to its own `connect_resize` and, when
//! the allocation changes, sends a [`lmux_wayland_host::HostCommand::ResizeToplevel`]
//! back to the compositor thread so the satellite client re-allocates
//! its buffer at the new size on its next draw.
//!
//! Input routing (Task #11) attaches here too, but lives in its own
//! follow-up module so this file stays focused on display + geometry.

use std::cell::{Cell, RefCell};
use std::os::fd::AsRawFd;
use std::rc::Rc;

use async_channel::Sender;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    EventControllerKey, EventControllerMotion, EventControllerScroll, EventControllerScrollFlags,
    Frame, GestureClick, Overlay, Picture,
};

use lmux_wayland_host::{DmabufFrame, HostCommand, SurfaceId};

use crate::layout::PaneId;
use lmux_config::FocusMode;

use crate::pane::FocusCallback;

// Linux evdev button codes. Wayland speaks evdev, GTK speaks
// 1=primary / 2=middle / 3=secondary, so we translate.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;

/// Hard floor on the configure we push to clients — smaller than this and
/// xdg-shell clients either reject or ship garbage (e.g. browser title
/// bars assume at least ~100px). Matches what KWin does.
const MIN_CONFIGURE: (u32, u32) = (32, 32);

/// A satellite pane. Terminal panes render a PTY; satellite panes render
/// a nested-wayland toplevel's latest frame.
pub struct SatelliteWidget {
    pane_id: PaneId,
    surface_id: SurfaceId,
    frame: Frame,
    /// Outermost child of `frame`. The base layer is [`picture`] (the
    /// toplevel surface); overlay children are the popup `Picture`s
    /// tracked in [`popups`], positioned by their margin_start /
    /// margin_top in parent-surface coordinates.
    overlay: Overlay,
    picture: Picture,
    inner: Rc<RefCell<Inner>>,
    cmd_tx: Sender<HostCommand>,
    /// Flipped to true when `HostEvent::ToplevelClosed` arrives for this
    /// surface id. Mirrors `TerminalPane::has_exited` so `AppState` can
    /// treat satellite death uniformly with terminal-child exits.
    closed: Rc<Cell<bool>>,
    /// Cockpit-side focus callback. Invoked from the Picture's focus
    /// controller so `AppState.focused` tracks satellites the same way
    /// it tracks terminals — otherwise window-level shortcuts can't
    /// tell which pane the user is actually typing into.
    focus_cb: Rc<RefCell<Option<FocusCallback>>>,
    /// Shared focus-mode cell, populated when `attach_focus_callback` runs.
    /// `install_pointer_controllers`' enter handler reads this on every
    /// pointer enter to decide whether to grab focus (click vs hover mode).
    focus_mode: Rc<RefCell<Option<crate::pane::FocusModeCell>>>,
    /// Active popups keyed by their surface id. The `Picture` is an
    /// overlay child of [`overlay`]; its margin_start / margin_top encode
    /// the popup's position and its size_request encodes the popup's
    /// size. We keep the handle so we can call `set_paintable` on every
    /// frame without walking the widget tree.
    popups: Rc<RefCell<std::collections::HashMap<SurfaceId, Picture>>>,
}

struct Inner {
    /// Most recent `(width, height)` we told the satellite to resize to.
    /// Used to debounce the `ResizeToplevel` command — GTK fires
    /// `connect_resize` for every intermediate allocation during a
    /// divider drag, but the client only cares about the final size.
    last_configured: Option<(u32, u32)>,
    /// Size of the most recently pushed frame. Drives widget→surface
    /// coordinate translation for pointer events: with ContentFit::ScaleDown
    /// the texture is centered and may be smaller than the pane, so GTK
    /// widget coordinates don't match what the client sees on its surface.
    last_frame_size: Option<(u32, u32)>,
    /// Human label shown in the pane chrome. Updated by
    /// `ToplevelTitleChanged` events.
    title: Option<String>,
    /// Reported by the client. Not used for display yet, but kept so
    /// the sidebar can group satellites by app in a later pass.
    app_id: Option<String>,
}

/// Map GTK widget-local coordinates onto the client's surface coordinates,
/// accounting for `ContentFit::ScaleDown` (texture drawn 1:1 when it fits,
/// proportionally scaled down when it's bigger, centered either way).
/// Returns the unchanged input when we don't know the frame size yet — the
/// worst case is a mis-click on the very first pointer event, which
/// self-corrects after the next frame.
fn map_widget_to_surface(
    widget_w: i32,
    widget_h: i32,
    frame: Option<(u32, u32)>,
    x: f64,
    y: f64,
) -> (f64, f64) {
    let (Some((tw, th)), true) = (frame, widget_w > 0 && widget_h > 0) else {
        return (x, y);
    };
    if tw == 0 || th == 0 {
        return (x, y);
    }
    let (tw, th) = (tw as f64, th as f64);
    let (ww, wh) = (widget_w as f64, widget_h as f64);
    // ScaleDown: shrink to fit if oversized, otherwise draw 1:1.
    let scale = (ww / tw).min(wh / th).min(1.0);
    let painted_w = tw * scale;
    let painted_h = th * scale;
    let ox = (ww - painted_w) / 2.0;
    let oy = (wh - painted_h) / 2.0;
    ((x - ox) / scale, (y - oy) / scale)
}

impl SatelliteWidget {
    /// Build a fresh satellite widget for `surface_id`. The caller
    /// supplies `title` + `app_id` from the triggering
    /// [`HostEvent::ToplevelCreated`] so the pane chrome has a label
    /// before the first frame arrives.
    pub fn new(
        pane_id: PaneId,
        surface_id: SurfaceId,
        title: Option<String>,
        app_id: Option<String>,
        cmd_tx: Sender<HostCommand>,
    ) -> Self {
        // ScaleDown (not Fill) so the brief window between the client's
        // first commit (at its preferred small size) and our follow-up
        // ResizeToplevel doesn't show a blurry stretched bitmap. Once the
        // client redraws at the configured size, the texture matches the
        // pane allocation exactly and ScaleDown is a no-op.
        let picture = Picture::builder()
            .hexpand(true)
            .vexpand(true)
            .content_fit(gtk4::ContentFit::ScaleDown)
            .build();

        let overlay = Overlay::builder().hexpand(true).vexpand(true).build();
        overlay.set_child(Some(&picture));

        let frame = Frame::builder().hexpand(true).vexpand(true).build();
        frame.set_child(Some(&overlay));
        frame.add_css_class("pane");
        frame.add_css_class("satellite");

        let inner = Rc::new(RefCell::new(Inner {
            last_configured: None,
            last_frame_size: None,
            title,
            app_id,
        }));

        // The Picture must be focusable so EventControllerKey actually
        // receives input. Without this `grab_focus` is a no-op.
        picture.set_focusable(true);

        let widget = Self {
            pane_id,
            surface_id,
            frame,
            overlay,
            picture,
            inner,
            cmd_tx,
            closed: Rc::new(Cell::new(false)),
            focus_cb: Rc::new(RefCell::new(None)),
            focus_mode: Rc::new(RefCell::new(None)),
            popups: Rc::new(RefCell::new(std::collections::HashMap::new())),
        };
        widget.install_resize_handler();
        widget.install_pointer_controllers();
        widget.install_scroll_controller();
        widget.install_key_controller();
        widget
    }

    #[allow(dead_code)]
    pub fn surface_id(&self) -> SurfaceId {
        self.surface_id
    }

    pub fn pane_id(&self) -> PaneId {
        self.pane_id
    }

    /// Called by the host-event dispatcher when the client destroys the
    /// toplevel. After this, `has_exited()` returns true and
    /// `AppState::close_focused` (or the dispatcher directly) removes the
    /// pane from the layout.
    pub fn mark_closed(&self) {
        self.closed.set(true);
    }

    pub fn has_exited(&self) -> bool {
        self.closed.get()
    }

    /// Move GTK keyboard focus onto the child picture so subsequent key
    /// events route to the satellite's `wl_keyboard`. Counterpart to
    /// `TerminalPane::grab_focus`.
    pub fn grab_focus(&self) {
        self.picture.grab_focus();
    }

    /// Register the cockpit's focus callback so window-level shortcuts
    /// can distinguish a focused satellite from a focused terminal.
    /// Called by `Pane::attach_controllers` after the satellite is
    /// spliced into the layout.
    pub fn attach_focus_callback(&self, cb: FocusCallback, focus_mode: crate::pane::FocusModeCell) {
        *self.focus_cb.borrow_mut() = Some(cb);
        *self.focus_mode.borrow_mut() = Some(focus_mode);
    }

    /// The GTK subtree to plug into the pane tree. Same shape as
    /// `Pane::widget()` so `AppState` can treat both uniformly.
    pub fn widget(&self) -> &Frame {
        &self.frame
    }

    /// Attach an xdg_popup (menu/dropdown/tooltip) as an overlay on top
    /// of this satellite's main Picture. Position is encoded as
    /// margin_start/margin_top; size as a `size_request` so the popup
    /// sticks to its positioner-resolved geometry. Popups with halign/
    /// valign = start paint at (margin_start, margin_top) in the
    /// overlay's coord space — which is the parent surface's
    /// window-geometry space, exactly what the xdg_positioner gives us.
    pub fn attach_popup(&self, popup_id: SurfaceId, x: i32, y: i32, width: u32, height: u32) {
        let pic = Picture::builder()
            .hexpand(false)
            .vexpand(false)
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Start)
            .content_fit(gtk4::ContentFit::Fill)
            .margin_start(x.max(0))
            .margin_top(y.max(0))
            .build();
        pic.set_size_request(width as i32, height as i32);
        pic.set_focusable(true);
        pic.add_css_class("satellite__popup");
        Self::install_popup_input(&pic, popup_id, &self.cmd_tx);
        self.overlay.add_overlay(&pic);
        self.popups.borrow_mut().insert(popup_id, pic);
    }

    /// Attach pointer / scroll / key controllers to a popup overlay
    /// Picture so child toplevels (xdg_popup + set_parent dialogs) are
    /// actually clickable. Mirrors [`install_pointer_controllers`] /
    /// [`install_scroll_controller`] / [`install_key_controller`] but
    /// addresses events at `popup_id` rather than the parent's surface id.
    fn install_popup_input(pic: &Picture, popup_id: SurfaceId, cmd_tx: &Sender<HostCommand>) {
        let id = popup_id;

        let motion = EventControllerMotion::new();
        let tx = cmd_tx.clone();
        motion.connect_motion(move |_c, x, y| {
            let _ = tx.send_blocking(HostCommand::PointerMotion { id, x, y });
        });
        let tx = cmd_tx.clone();
        motion.connect_enter(move |_c, x, y| {
            let _ = tx.send_blocking(HostCommand::PointerMotion { id, x, y });
        });
        let tx = cmd_tx.clone();
        motion.connect_leave(move |_c| {
            let _ = tx.send_blocking(HostCommand::PointerLeave { id });
        });
        pic.add_controller(motion);

        for (gtk_button, evdev) in [
            (gdk::BUTTON_PRIMARY, BTN_LEFT),
            (gdk::BUTTON_SECONDARY, BTN_RIGHT),
            (gdk::BUTTON_MIDDLE, BTN_MIDDLE),
        ] {
            let click = GestureClick::new();
            click.set_button(gtk_button);
            let pic_focus = pic.clone();
            let tx = cmd_tx.clone();
            click.connect_pressed(move |_g, _n, _x, _y| {
                pic_focus.grab_focus();
                let _ = tx.send_blocking(HostCommand::PointerButton {
                    id,
                    button: evdev,
                    pressed: true,
                });
            });
            let tx = cmd_tx.clone();
            click.connect_released(move |_g, _n, _x, _y| {
                let _ = tx.send_blocking(HostCommand::PointerButton {
                    id,
                    button: evdev,
                    pressed: false,
                });
            });
            pic.add_controller(click);
        }

        let scroll = EventControllerScroll::new(
            EventControllerScrollFlags::VERTICAL | EventControllerScrollFlags::HORIZONTAL,
        );
        let tx = cmd_tx.clone();
        scroll.connect_scroll(move |_c, dx, dy| {
            let _ = tx.send_blocking(HostCommand::PointerAxis { id, dx, dy });
            glib::Propagation::Stop
        });
        pic.add_controller(scroll);

        let key = EventControllerKey::new();
        let tx = cmd_tx.clone();
        key.connect_key_pressed(move |_c, _kv, hw, _m| {
            let evdev = hw.saturating_sub(8);
            let _ = tx.send_blocking(HostCommand::KeyInput {
                id,
                evdev_code: evdev,
                pressed: true,
            });
            glib::Propagation::Stop
        });
        let tx = cmd_tx.clone();
        key.connect_key_released(move |_c, _kv, hw, _m| {
            let evdev = hw.saturating_sub(8);
            let _ = tx.send_blocking(HostCommand::KeyInput {
                id,
                evdev_code: evdev,
                pressed: false,
            });
        });
        pic.add_controller(key);

        let focus = gtk4::EventControllerFocus::new();
        let tx = cmd_tx.clone();
        focus.connect_enter(move |_c| {
            let _ = tx.send_blocking(HostCommand::KeyboardFocus { id: Some(id) });
        });
        pic.add_controller(focus);
    }

    /// Re-place an already-attached popup (xdg_positioner reposition).
    pub fn reposition_popup(&self, popup_id: SurfaceId, x: i32, y: i32, width: u32, height: u32) {
        if let Some(pic) = self.popups.borrow().get(&popup_id) {
            pic.set_margin_start(x.max(0));
            pic.set_margin_top(y.max(0));
            pic.set_size_request(width as i32, height as i32);
        }
    }

    /// Remove a popup's overlay Picture. Called when the client destroys
    /// the xdg_popup (menu closed / client crashed).
    pub fn detach_popup(&self, popup_id: SurfaceId) {
        if let Some(pic) = self.popups.borrow_mut().remove(&popup_id) {
            self.overlay.remove_overlay(&pic);
        }
    }

    /// Route an shm frame into a popup's Picture. Mirrors
    /// [`push_frame`](Self::push_frame) but targets the overlay layer
    /// rather than the base texture. No-op if the popup is unknown
    /// (e.g. the cockpit hasn't seen its PopupCreated event yet — racy
    /// but harmless, the next commit will find it).
    pub fn push_popup_frame(&self, popup_id: SurfaceId, width: u32, height: u32, rgb: Vec<u8>) {
        if width == 0 || height == 0 || rgb.is_empty() {
            return;
        }
        let expected = (width as usize) * (height as usize) * 3;
        if rgb.len() != expected {
            return;
        }
        let stride = (width as usize) * 3;
        let bytes = glib::Bytes::from_owned(rgb);
        let texture = gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk::MemoryFormat::R8g8b8,
            &bytes,
            stride,
        );
        if let Some(pic) = self.popups.borrow().get(&popup_id) {
            pic.set_paintable(Some(&texture));
        }
    }

    /// Dmabuf counterpart of [`push_popup_frame`](Self::push_popup_frame).
    pub fn push_popup_dmabuf_frame(&self, popup_id: SurfaceId, frame: DmabufFrame) {
        if frame.width == 0 || frame.height == 0 {
            return;
        }
        let builder = gdk::DmabufTextureBuilder::new();
        builder.set_width(frame.width);
        builder.set_height(frame.height);
        builder.set_fourcc(frame.fourcc);
        builder.set_modifier(frame.modifier);
        builder.set_n_planes(1);
        builder.set_fd(0, frame.fd.as_raw_fd());
        builder.set_stride(0, frame.stride);
        builder.set_offset(0, frame.offset);
        let fd = frame.fd;
        let texture = unsafe {
            builder.build_with_release_func(move || {
                drop(fd);
            })
        };
        match texture {
            Ok(tex) => {
                if let Some(pic) = self.popups.borrow().get(&popup_id) {
                    pic.set_paintable(Some(&tex));
                }
            }
            Err(err) => {
                tracing::warn!(
                    popup = %popup_id,
                    fourcc = frame.fourcc,
                    modifier = frame.modifier,
                    error = %err,
                    "satellite: popup dmabuf import failed"
                );
            }
        }
    }

    /// Route a freshly-decoded frame into the displayed texture. Called
    /// from the host-event dispatcher on the GTK main thread.
    pub fn push_frame(&self, width: u32, height: u32, rgb: Vec<u8>) {
        if width == 0 || height == 0 || rgb.is_empty() {
            return;
        }
        let expected = (width as usize) * (height as usize) * 3;
        if rgb.len() != expected {
            tracing::warn!(
                id = %self.surface_id,
                width,
                height,
                got = rgb.len(),
                expected,
                "satellite: malformed frame (stride mismatch) — dropping"
            );
            return;
        }
        let stride = (width as usize) * 3;
        let bytes = glib::Bytes::from_owned(rgb);
        let texture = gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk::MemoryFormat::R8g8b8,
            &bytes,
            stride,
        );
        self.inner.borrow_mut().last_frame_size = Some((width, height));
        self.picture.set_paintable(Some(&texture));
    }

    /// Route a dmabuf-backed frame into the displayed texture. The fd is
    /// imported straight into a `gdk::DmabufTexture` so the GPU holds the
    /// pixel data — no CPU memcpy. Single-plane only; multi-plane / YUV
    /// rides v0.3. If GTK rejects the format (e.g. no EGL/Vulkan import
    /// path for this fourcc+modifier on the current driver), we drop the
    /// frame silently and let the compositor fall back to shm on the
    /// client's next commit.
    pub fn push_dmabuf_frame(&self, frame: DmabufFrame) {
        if frame.width == 0 || frame.height == 0 {
            return;
        }
        let (fw, fh) = (frame.width, frame.height);
        let builder = gdk::DmabufTextureBuilder::new();
        builder.set_width(frame.width);
        builder.set_height(frame.height);
        builder.set_fourcc(frame.fourcc);
        builder.set_modifier(frame.modifier);
        builder.set_n_planes(1);
        builder.set_fd(0, frame.fd.as_raw_fd());
        builder.set_stride(0, frame.stride);
        builder.set_offset(0, frame.offset);
        // GTK does not dup the fd — it needs it kept open until the
        // release callback fires. Move the OwnedFd into the closure so
        // `drop` there closes it once the texture is gone.
        let fd = frame.fd;
        let texture = unsafe {
            builder.build_with_release_func(move || {
                drop(fd);
            })
        };
        match texture {
            Ok(tex) => {
                self.inner.borrow_mut().last_frame_size = Some((fw, fh));
                self.picture.set_paintable(Some(&tex));
            }
            Err(err) => {
                tracing::warn!(
                    id = %self.surface_id,
                    fourcc = frame.fourcc,
                    modifier = frame.modifier,
                    error = %err,
                    "satellite: dmabuf import failed — dropping frame"
                );
            }
        }
    }

    /// Update the cached title (fired by `ToplevelTitleChanged`). The
    /// actual label rendering belongs to whatever pane-chrome widget
    /// wraps us; for now we just stash the string.
    pub fn set_title(&self, title: Option<String>) {
        self.inner.borrow_mut().title = title;
    }

    #[allow(dead_code)]
    pub fn title(&self) -> Option<String> {
        self.inner.borrow().title.clone()
    }

    pub fn set_app_id(&self, app_id: Option<String>) {
        self.inner.borrow_mut().app_id = app_id;
    }

    #[allow(dead_code)]
    pub fn app_id(&self) -> Option<String> {
        self.inner.borrow().app_id.clone()
    }

    /// Show/hide for anchor workspace swap. Uses `visible` (cheap) rather
    /// than reparenting so focus state stays stable.
    #[allow(dead_code)]
    pub fn set_visible(&self, visible: bool) {
        self.frame.set_visible(visible);
    }

    /// Apply a freedesktop/CSS cursor name ("default", "pointer", "text",
    /// "ew-resize", "nwse-resize", …) to the satellite's Picture. GTK
    /// resolves the name against the active cursor theme on the host, so
    /// IntelliJ's "resize divider" → "ew-resize" actually turns into the
    /// same cursor KWin would show for a native window.
    pub fn set_cursor_shape(&self, name: &str) {
        let display = gdk::Display::default();
        let cursor = display.as_ref().and_then(|d| {
            gdk::Cursor::from_name(name, None).or_else(|| {
                // Fall back to the display's default cursor when the
                // theme lacks the requested shape (e.g. "none" on a theme
                // that doesn't define it) — better than leaving the
                // previous cursor stuck.
                let _ = d;
                gdk::Cursor::from_name("default", None)
            })
        });
        self.picture.set_cursor(cursor.as_ref());
        for pic in self.popups.borrow().values() {
            pic.set_cursor(cursor.as_ref());
        }
    }

    /// Apply a custom cursor texture (sent by the client via the legacy
    /// `wl_pointer.set_cursor(surface)` path — JetBrains uses this for
    /// resize/I-beam, not wp_cursor_shape_device_v1). The texture is
    /// fed straight into a `gdk::Cursor::from_texture`, which GTK draws
    /// at the hotspot offset whenever the pointer is inside this Picture.
    pub fn set_cursor_bitmap(
        &self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        hotspot_x: i32,
        hotspot_y: i32,
    ) {
        if width == 0 || height == 0 || rgba.is_empty() {
            return;
        }
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() != expected {
            tracing::warn!(
                width,
                height,
                got = rgba.len(),
                expected,
                "satellite: malformed cursor bitmap — ignoring"
            );
            return;
        }
        let stride = (width as usize) * 4;
        let bytes = glib::Bytes::from_owned(rgba);
        // Wayland ARGB8888 is premultiplied alpha per spec. GTK's
        // R8g8b8a8Premultiplied matches that exactly so the transparent
        // border around the cursor renders as transparent — not as the
        // black rectangle you get if you feed straight (non-premul) RGBA
        // through `R8g8b8a8`.
        let texture = gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk::MemoryFormat::R8g8b8a8Premultiplied,
            &bytes,
            stride,
        );
        let fallback = gdk::Cursor::from_name("default", None);
        let cursor = gdk::Cursor::from_texture(&texture, hotspot_x, hotspot_y, fallback.as_ref());
        self.picture.set_cursor(Some(&cursor));
        for pic in self.popups.borrow().values() {
            pic.set_cursor(Some(&cursor));
        }
    }

    /// Ask the nested compositor to send a close event. Used by
    /// Ctrl+B x on a satellite pane.
    pub fn request_close(&self) {
        let _ = self.cmd_tx.send_blocking(HostCommand::CloseToplevel {
            id: self.surface_id,
        });
    }

    fn install_pointer_controllers(&self) {
        // Motion + enter/leave
        let motion = EventControllerMotion::new();
        let cmd_tx = self.cmd_tx.clone();
        let id = self.surface_id;
        let pic = self.picture.clone();
        let inner = self.inner.clone();
        motion.connect_motion(move |_c, x, y| {
            let frame = inner.borrow().last_frame_size;
            let (sx, sy) = map_widget_to_surface(pic.width(), pic.height(), frame, x, y);
            let _ = cmd_tx.send_blocking(HostCommand::PointerMotion { id, x: sx, y: sy });
        });
        let cmd_tx_enter = self.cmd_tx.clone();
        let pic_enter = self.picture.clone();
        let inner_enter = self.inner.clone();
        let focus_mode_enter = self.focus_mode.clone();
        motion.connect_enter(move |_c, x, y| {
            // GTK fires `enter` before the first `motion`, so seed the
            // pointer position so the first click lands at the cursor.
            let frame = inner_enter.borrow().last_frame_size;
            let (sx, sy) =
                map_widget_to_surface(pic_enter.width(), pic_enter.height(), frame, x, y);
            let _ = cmd_tx_enter.send_blocking(HostCommand::PointerMotion { id, x: sx, y: sy });
            // focus-follows-mouse: grabbing focus on the Picture fires the
            // EventControllerFocus::enter handler below, which in turn
            // sends KeyboardFocus to the host and notifies AppState via
            // `focus_cb`. So we just need to trigger grab_focus here.
            if let Some(mode) = focus_mode_enter.borrow().as_ref() {
                if mode.get() == FocusMode::Hover {
                    pic_enter.grab_focus();
                }
            }
        });
        let cmd_tx_leave = self.cmd_tx.clone();
        motion.connect_leave(move |_c| {
            let _ = cmd_tx_leave.send_blocking(HostCommand::PointerLeave { id });
        });
        self.picture.add_controller(motion);

        // Click — bind one gesture per supported button so the
        // controller routes button-distinct events without us having
        // to introspect a single any-button gesture.
        for (gtk_button, evdev) in [
            (gdk::BUTTON_PRIMARY, BTN_LEFT),
            (gdk::BUTTON_SECONDARY, BTN_RIGHT),
            (gdk::BUTTON_MIDDLE, BTN_MIDDLE),
        ] {
            let click = GestureClick::new();
            click.set_button(gtk_button);
            let pic = self.picture.clone();
            let cmd_tx = self.cmd_tx.clone();
            let inner = self.inner.clone();
            click.connect_pressed(move |_g, _n, x, y| {
                pic.grab_focus();
                // Seed a motion at the click site so wayland sees the
                // pointer at the exact surface coord before the button
                // press — otherwise the button lands wherever the last
                // motion event was and click-to-icon misses by pixels.
                let frame = inner.borrow().last_frame_size;
                let (sx, sy) = map_widget_to_surface(pic.width(), pic.height(), frame, x, y);
                let _ = cmd_tx.send_blocking(HostCommand::PointerMotion { id, x: sx, y: sy });
                let _ = cmd_tx.send_blocking(HostCommand::PointerButton {
                    id,
                    button: evdev,
                    pressed: true,
                });
            });
            let cmd_tx = self.cmd_tx.clone();
            click.connect_released(move |_g, _n, _x, _y| {
                let _ = cmd_tx.send_blocking(HostCommand::PointerButton {
                    id,
                    button: evdev,
                    pressed: false,
                });
            });
            self.picture.add_controller(click);
        }
    }

    fn install_scroll_controller(&self) {
        let scroll = EventControllerScroll::new(
            EventControllerScrollFlags::VERTICAL | EventControllerScrollFlags::HORIZONTAL,
        );
        let cmd_tx = self.cmd_tx.clone();
        let id = self.surface_id;
        scroll.connect_scroll(move |_c, dx, dy| {
            let _ = cmd_tx.send_blocking(HostCommand::PointerAxis { id, dx, dy });
            glib::Propagation::Stop
        });
        self.picture.add_controller(scroll);
    }

    fn install_key_controller(&self) {
        let key = EventControllerKey::new();
        let cmd_tx_press = self.cmd_tx.clone();
        let id = self.surface_id;
        key.connect_key_pressed(move |_c, _keyval, hw_keycode, _mods| {
            // GTK gives an XKB keycode (= evdev + 8); the host expects evdev.
            let evdev = hw_keycode.saturating_sub(8);
            let _ = cmd_tx_press.send_blocking(HostCommand::KeyInput {
                id,
                evdev_code: evdev,
                pressed: true,
            });
            glib::Propagation::Stop
        });
        let cmd_tx_release = self.cmd_tx.clone();
        key.connect_key_released(move |_c, _keyval, hw_keycode, _mods| {
            let evdev = hw_keycode.saturating_sub(8);
            let _ = cmd_tx_release.send_blocking(HostCommand::KeyInput {
                id,
                evdev_code: evdev,
                pressed: false,
            });
        });
        self.picture.add_controller(key);

        // Focus enter/leave → keyboard focus change. We listen on the
        // Picture itself; GTK delivers focus events when the picture
        // becomes the GtkWindow's focused widget (which happens via
        // `grab_focus()` from the click handlers above, or when the
        // user tabs into the pane).
        let focus = gtk4::EventControllerFocus::new();
        let cmd_tx_in = self.cmd_tx.clone();
        let focus_cb_in = self.focus_cb.clone();
        let pane_id = self.pane_id;
        focus.connect_enter(move |_c| {
            let _ = cmd_tx_in.send_blocking(HostCommand::KeyboardFocus { id: Some(id) });
            if let Some(cb) = focus_cb_in.borrow().as_ref() {
                cb(pane_id);
            }
        });
        let cmd_tx_out = self.cmd_tx.clone();
        focus.connect_leave(move |_c| {
            let _ = cmd_tx_out.send_blocking(HostCommand::KeyboardFocus { id: None });
        });
        self.picture.add_controller(focus);
    }

    fn install_resize_handler(&self) {
        let inner = self.inner.clone();
        let cmd_tx = self.cmd_tx.clone();
        let id = self.surface_id;
        // GTK4 has no public `size_allocate` signal on generic widgets.
        // Picture in particular doesn't implement `DrawingAreaExt::connect_resize`.
        // A frame-clock tick callback is the idiomatic way to watch the
        // allocation — it fires once per frame, we cheap-compare against
        // the last configured size, and only post `ResizeToplevel` when
        // the allocation actually changed.
        let last_logged: Rc<Cell<(i32, i32)>> = Rc::new(Cell::new((-1, -1)));
        self.picture.add_tick_callback(move |pic, _clock| {
            let w = pic.width();
            let h = pic.height();
            // Log every change in raw pic dimensions so we can tell whether
            // GTK is even reporting the new allocation after a paned drag,
            // independent of our MIN_CONFIGURE clamp / cache logic below.
            if last_logged.get() != (w, h) {
                last_logged.set((w, h));
                tracing::debug!(
                    target: "lmux::satellite::resize",
                    ?id, w, h, "tick: picture allocation observed",
                );
            }
            if w <= 0 || h <= 0 {
                return glib::ControlFlow::Continue;
            }
            let (w_u, h_u) = (
                (w as u32).max(MIN_CONFIGURE.0),
                (h as u32).max(MIN_CONFIGURE.1),
            );
            let mut b = inner.borrow_mut();
            if b.last_configured == Some((w_u, h_u)) {
                return glib::ControlFlow::Continue;
            }
            b.last_configured = Some((w_u, h_u));
            drop(b);
            tracing::debug!(
                target: "lmux::satellite::resize",
                ?id, w = w_u, h = h_u,
                "tick: posting ResizeToplevel",
            );
            let _ = cmd_tx.send_blocking(HostCommand::ResizeToplevel {
                id,
                width: w_u,
                height: h_u,
            });
            glib::ControlFlow::Continue
        });
    }
}
