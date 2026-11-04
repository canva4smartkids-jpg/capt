//! X11 access via x11rb: RandR monitors, EWMH/ICCCM windows, MIT-SHM / plain GetImage
//! grabs and XFixes cursor overlay.

use capture_core::{CaptureError, CaptureErrorCode, CaptureResult, VideoFrame};
use x11rb::connection::{Connection, RequestConnection};
use x11rb::errors::ReplyError;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{self, AtomEnum, ConnectionExt as _, ImageFormat, ImageOrder, MapState, Window};
use x11rb::protocol::ErrorKind;
use x11rb::rust_connection::RustConnection;

pub const WHOLE_SCREEN_ID: &str = "all";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone)]
pub struct Monitor {
    pub id: String,
    pub rect: XRect,
    pub primary: bool,
}

#[derive(Debug, Clone)]
pub struct WinInfo {
    pub id: Window,
    pub title: String,
    pub class: Option<String>,
    pub rect: XRect,
}

struct Atoms {
    net_client_list_stacking: u32,
    net_client_list: u32,
    net_wm_name: u32,
    utf8_string: u32,
    wm_state: u32,
}

pub struct X11 {
    pub conn: RustConnection,
    pub root: Window,
    pub has_randr: bool,
    pub has_xfixes: bool,
    pub has_shm: bool,
    atoms: Atoms,
}

/// Outcome of a window lookup.
pub enum WinState {
    Visible(XRect),
    /// Exists but is not viewable (minimized / unmapped).
    Hidden,
    Gone,
}

fn x_err(context: &str, e: impl std::fmt::Display) -> CaptureError {
    CaptureError::internal(format!("{context}: {e}"))
}

fn is_gone(e: &ReplyError) -> bool {
    matches!(e, ReplyError::X11Error(x) if matches!(x.error_kind, ErrorKind::Window | ErrorKind::Drawable | ErrorKind::Match))
}

/// A System V shared memory segment attached to the X server (MIT-SHM).
pub struct ShmSeg {
    seg: shm::Seg,
    addr: *mut u8,
    size: usize,
}

// The mapping is process-wide memory; only the owning grab thread touches it.
unsafe impl Send for ShmSeg {}

impl Drop for ShmSeg {
    fn drop(&mut self) {
        // The segment was marked IPC_RMID right after attaching, so it disappears once the
        // X server (on disconnect) and we have detached.
        unsafe {
            libc::shmdt(self.addr as *const libc::c_void);
        }
    }
}

impl X11 {
    pub fn connect() -> CaptureResult<X11> {
        let display = std::env::var("DISPLAY").unwrap_or_default();
        let (conn, screen_num) = x11rb::connect(None).map_err(|e| {
            CaptureError::unsupported(format!("cannot connect to an X11 display (DISPLAY='{display}'): {e}. Wayland-only sessions need XWayland."))
        })?;
        let (root, depth_ok) = {
            let setup = conn.setup();
            let screen = setup.roots.get(screen_num).ok_or_else(|| CaptureError::internal("X screen not found"))?;
            let depth = screen.root_depth;
            let bpp32 = setup.pixmap_formats.iter().any(|f| f.depth == depth && f.bits_per_pixel == 32);
            let lsb = setup.image_byte_order == ImageOrder::LSB_FIRST;
            let masks = screen
                .allowed_depths
                .iter()
                .flat_map(|d| d.visuals.iter())
                .find(|v| v.visual_id == screen.root_visual)
                .map(|v| v.red_mask == 0xff0000 && v.green_mask == 0xff00 && v.blue_mask == 0xff)
                .unwrap_or(false);
            (screen.root, (depth == 24 || depth == 32) && bpp32 && lsb && masks)
        };
        if !depth_ok {
            return Err(CaptureError::unsupported("X screen pixel format is not 24/32-bit little-endian TrueColor; capture is not supported"));
        }
        let has_randr = conn.extension_information(x11rb::protocol::randr::X11_EXTENSION_NAME).ok().flatten().is_some()
            && conn
                .randr_query_version(1, 5)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| r.major_version > 1 || (r.major_version == 1 && r.minor_version >= 5))
                .unwrap_or(false);
        let has_xfixes = conn.extension_information(x11rb::protocol::xfixes::X11_EXTENSION_NAME).ok().flatten().is_some()
            && conn.xfixes_query_version(4, 0).ok().and_then(|c| c.reply().ok()).is_some();
        let has_shm = conn.extension_information(shm::X11_EXTENSION_NAME).ok().flatten().is_some() && conn.shm_query_version().ok().and_then(|c| c.reply().ok()).is_some();
        let intern = |name: &[u8]| -> CaptureResult<u32> {
            Ok(conn.intern_atom(false, name).map_err(|e| x_err("intern_atom", e))?.reply().map_err(|e| x_err("intern_atom", e))?.atom)
        };
        let atoms = Atoms {
            net_client_list_stacking: intern(b"_NET_CLIENT_LIST_STACKING")?,
            net_client_list: intern(b"_NET_CLIENT_LIST")?,
            net_wm_name: intern(b"_NET_WM_NAME")?,
            utf8_string: intern(b"UTF8_STRING")?,
            wm_state: intern(b"WM_STATE")?,
        };
        Ok(X11 { conn, root, has_randr, has_xfixes, has_shm, atoms })
    }

    /// Current root window size (changes with RandR reconfiguration).
    pub fn root_size(&self) -> CaptureResult<(u32, u32)> {
        let g = self.conn.get_geometry(self.root).map_err(|e| x_err("get_geometry", e))?.reply().map_err(|e| x_err("get_geometry", e))?;
        Ok((g.width as u32, g.height as u32))
    }

    fn atom_name(&self, atom: u32) -> Option<String> {
        let r = self.conn.get_atom_name(atom).ok()?.reply().ok()?;
        Some(String::from_utf8_lossy(&r.name).to_string())
    }

    /// RandR 1.5 monitors (primary first). Ids are the monitor names (e.g. "DP-1").
    pub fn monitors(&self) -> Vec<Monitor> {
        let mut out: Vec<Monitor> = Vec::new();
        if !self.has_randr {
            return out;
        }
        let Some(reply) = self.conn.randr_get_monitors(self.root, true).ok().and_then(|c| c.reply().ok()) else { return out };
        for (i, m) in reply.monitors.iter().enumerate() {
            let mut id = self.atom_name(m.name).filter(|n| !n.is_empty()).unwrap_or_else(|| format!("monitor-{i}"));
            if id == WHOLE_SCREEN_ID || out.iter().any(|o| o.id == id) {
                id = format!("{id}-{i}");
            }
            out.push(Monitor { id, rect: XRect { x: m.x as i32, y: m.y as i32, w: m.width as u32, h: m.height as u32 }, primary: m.primary });
        }
        out.sort_by_key(|m| !m.primary);
        out
    }

    fn prop_bytes(&self, win: Window, prop: u32, ty: u32) -> Option<Vec<u8>> {
        let r = self.conn.get_property(false, win, prop, ty, 0, 1 << 16).ok()?.reply().ok()?;
        (!r.value.is_empty()).then_some(r.value)
    }

    fn window_title(&self, win: Window) -> Option<String> {
        self.prop_bytes(win, self.atoms.net_wm_name, self.atoms.utf8_string)
            .or_else(|| self.prop_bytes(win, AtomEnum::WM_NAME.into(), AtomEnum::ANY.into()))
            .map(|b| String::from_utf8_lossy(&b).trim_end_matches('\0').trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn window_class(&self, win: Window) -> Option<String> {
        let b = self.prop_bytes(win, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())?;
        let parts: Vec<String> = b.split(|&c| c == 0).filter(|p| !p.is_empty()).map(|p| String::from_utf8_lossy(p).to_string()).collect();
        parts.get(1).or_else(|| parts.first()).cloned()
    }

    fn client_list(&self) -> Vec<Window> {
        for atom in [self.atoms.net_client_list_stacking, self.atoms.net_client_list] {
            if let Some(r) = self.conn.get_property(false, self.root, atom, AtomEnum::WINDOW, 0, 1 << 16).ok().and_then(|c| c.reply().ok()) {
                if let Some(v) = r.value32() {
                    let mut list: Vec<Window> = v.collect();
                    if !list.is_empty() {
                        list.reverse(); // stacking order is bottom-to-top; list topmost first
                        return list;
                    }
                }
            }
        }
        Vec::new()
    }

    /// Without an EWMH window manager: top-level children of the root (or the client inside
    /// a reparenting frame), topmost first.
    fn toplevel_fallback(&self) -> Vec<Window> {
        let Some(tree) = self.conn.query_tree(self.root).ok().and_then(|c| c.reply().ok()) else { return Vec::new() };
        let mut out = Vec::new();
        for &w in tree.children.iter().rev() {
            let Some(attrs) = self.conn.get_window_attributes(w).ok().and_then(|c| c.reply().ok()) else { continue };
            if attrs.override_redirect || attrs.map_state != MapState::VIEWABLE {
                continue;
            }
            if self.window_title(w).is_some() {
                out.push(w);
                continue;
            }
            if let Some(sub) = self.conn.query_tree(w).ok().and_then(|c| c.reply().ok()) {
                if let Some(&client) = sub.children.iter().find(|&&c| self.prop_bytes(c, self.atoms.wm_state, self.atoms.wm_state).is_some() || self.window_title(c).is_some()) {
                    out.push(client);
                }
            }
        }
        out
    }

    pub fn windows(&self) -> Vec<WinInfo> {
        let mut ids = self.client_list();
        if ids.is_empty() {
            ids = self.toplevel_fallback();
        }
        let mut out = Vec::new();
        for id in ids {
            let WinState::Visible(rect) = self.window_state(id) else { continue };
            if rect.w < 16 || rect.h < 16 {
                continue;
            }
            let Some(title) = self.window_title(id) else { continue };
            out.push(WinInfo { id, title, class: self.window_class(id), rect });
        }
        out
    }

    /// Window geometry in root coordinates.
    pub fn window_state(&self, win: Window) -> WinState {
        let attrs = match self.conn.get_window_attributes(win).map(|c| c.reply()) {
            Ok(Ok(a)) => a,
            Ok(Err(e)) if is_gone(&e) => return WinState::Gone,
            _ => return WinState::Gone,
        };
        if attrs.map_state != MapState::VIEWABLE {
            return WinState::Hidden;
        }
        let geom = match self.conn.get_geometry(win).map(|c| c.reply()) {
            Ok(Ok(g)) => g,
            _ => return WinState::Gone,
        };
        let tr = match self.conn.translate_coordinates(win, self.root, 0, 0).map(|c| c.reply()) {
            Ok(Ok(t)) => t,
            _ => return WinState::Gone,
        };
        WinState::Visible(XRect { x: tr.dst_x as i32, y: tr.dst_y as i32, w: geom.width as u32, h: geom.height as u32 })
    }

    /// Creates and attaches an MIT-SHM segment of `size` bytes, or `None` if SHM does not
    /// work (remote display, IPC namespace mismatch, ...).
    pub fn create_shm(&self, size: usize) -> Option<ShmSeg> {
        if !self.has_shm || size == 0 {
            return None;
        }
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if id < 0 {
                return None;
            }
            let addr = libc::shmat(id, std::ptr::null(), 0);
            if addr as isize == -1 {
                libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
                return None;
            }
            let seg_id = self.conn.generate_id().ok();
            let attached = seg_id.and_then(|seg| self.conn.shm_attach(seg, id as u32, false).ok()).and_then(|c| c.check().ok()).is_some();
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            match (attached, seg_id) {
                (true, Some(seg)) => Some(ShmSeg { seg, addr: addr as *mut u8, size }),
                _ => {
                    libc::shmdt(addr);
                    None
                }
            }
        }
    }

    /// Grabs a rect that lies inside the root window, BGRA with opaque alpha.
    fn grab_inside(&self, shm: Option<&ShmSeg>, r: XRect) -> Result<Vec<u8>, ReplyError> {
        let len = r.w as usize * r.h as usize * 4;
        if let Some(seg) = shm.filter(|s| s.size >= len) {
            let format: u8 = ImageFormat::Z_PIXMAP.into();
            let cookie = self.conn.shm_get_image(self.root, r.x as i16, r.y as i16, r.w as u16, r.h as u16, !0, format, seg.seg, 0)?;
            cookie.reply()?;
            let mut out = vec![0u8; len];
            unsafe { std::ptr::copy_nonoverlapping(seg.addr, out.as_mut_ptr(), len) };
            for px in out.chunks_exact_mut(4) {
                px[3] = 255;
            }
            return Ok(out);
        }
        let reply = self.conn.get_image(ImageFormat::Z_PIXMAP, self.root, r.x as i16, r.y as i16, r.w as u16, r.h as u16, !0)?.reply()?;
        let mut data = reply.data;
        data.truncate(len);
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        Ok(data)
    }

    /// Grabs `r` (root coordinates). Parts outside the screen are black.
    pub fn grab(&self, shm: Option<&ShmSeg>, r: XRect, show_cursor: bool) -> Result<VideoFrame, CaptureError> {
        if r.w == 0 || r.h == 0 {
            return Err(CaptureError::invalid_config("capture area is empty"));
        }
        let (rw, rh) = self.root_size()?;
        let x0 = r.x.max(0);
        let y0 = r.y.max(0);
        let x1 = (r.x + r.w as i32).min(rw as i32);
        let y1 = (r.y + r.h as i32).min(rh as i32);
        let map_err = |e: ReplyError| {
            if is_gone(&e) {
                CaptureError::new(CaptureErrorCode::SourceClosed, format!("capture source disappeared: {e}"))
            } else {
                x_err("GetImage", e)
            }
        };
        let mut data = if x1 <= x0 || y1 <= y0 {
            let mut v = vec![0u8; r.w as usize * r.h as usize * 4];
            v.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            v
        } else if (x0, y0, x1, y1) == (r.x, r.y, r.x + r.w as i32, r.y + r.h as i32) {
            self.grab_inside(shm, r).map_err(map_err)?
        } else {
            let inner = XRect { x: x0, y: y0, w: (x1 - x0) as u32, h: (y1 - y0) as u32 };
            let part = self.grab_inside(shm, inner).map_err(map_err)?;
            let mut v = vec![0u8; r.w as usize * r.h as usize * 4];
            v.chunks_exact_mut(4).for_each(|p| p[3] = 255);
            let (dx, dy) = ((x0 - r.x) as usize, (y0 - r.y) as usize);
            let row = inner.w as usize * 4;
            for y in 0..inner.h as usize {
                let dst = ((dy + y) * r.w as usize + dx) * 4;
                v[dst..dst + row].copy_from_slice(&part[y * row..(y + 1) * row]);
            }
            v
        };
        if show_cursor && self.has_xfixes {
            self.overlay_cursor(&mut data, r);
        }
        VideoFrame::new_bgra(r.w, r.h, r.w * 4, data)
    }

    /// Alpha-blends the current cursor image (premultiplied ARGB from XFixes).
    fn overlay_cursor(&self, data: &mut [u8], r: XRect) {
        let Some(c) = self.conn.xfixes_get_cursor_image().ok().and_then(|c| c.reply().ok()) else { return };
        let (cw, ch) = (c.width as i32, c.height as i32);
        let ox = c.x as i32 - c.xhot as i32 - r.x;
        let oy = c.y as i32 - c.yhot as i32 - r.y;
        for cy in 0..ch {
            let fy = oy + cy;
            if fy < 0 || fy >= r.h as i32 {
                continue;
            }
            for cx in 0..cw {
                let fx = ox + cx;
                if fx < 0 || fx >= r.w as i32 {
                    continue;
                }
                let Some(&argb) = c.cursor_image.get((cy * cw + cx) as usize) else { continue };
                let a = argb >> 24;
                if a == 0 {
                    continue;
                }
                let (sr, sg, sb) = ((argb >> 16) & 0xff, (argb >> 8) & 0xff, argb & 0xff);
                let i = ((fy as usize * r.w as usize) + fx as usize) * 4;
                let inv = 255 - a;
                data[i] = (sb + (data[i] as u32 * inv + 127) / 255).min(255) as u8;
                data[i + 1] = (sg + (data[i + 1] as u32 * inv + 127) / 255).min(255) as u8;
                data[i + 2] = (sr + (data[i + 2] as u32 * inv + 127) / 255).min(255) as u8;
            }
        }
    }
}

/// Parses window ids as produced by `list_sources` ("0x1a00003") or decimal.
pub fn parse_window_id(s: &str) -> Option<Window> {
    let t = s.trim();
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(hex) => u32::from_str_radix(hex, 16).ok(),
        None => t.parse().ok(),
    }
}

pub fn format_window_id(w: Window) -> String {
    format!("0x{w:x}")
}

#[allow(dead_code)]
fn _assert_xproto_used(_: xproto::Window) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_ids_roundtrip() {
        assert_eq!(parse_window_id("0x1a00003"), Some(0x1a00003));
        assert_eq!(parse_window_id("27262979"), Some(27262979));
        assert_eq!(parse_window_id("nope"), None);
        assert_eq!(format_window_id(0x1a00003), "0x1a00003");
    }
}
