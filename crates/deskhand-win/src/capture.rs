//! Window screenshots.
//!
//! Uses `PrintWindow` with full-content rendering, which captures the
//! window's own pixels even when other windows (including DeskHand's panel)
//! overlap it, and works for GPU-composited apps such as browsers.

use std::ffi::c_void;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::UI::WindowsAndMessaging::IsIconic;

use crate::error::{Result, WinError};
use crate::image::{RgbImage, bgra_to_rgb, downscale, encode_png, is_blank, to_base64};
use crate::window::{WindowId, frame_rect, live, window_rect};

/// `PW_RENDERFULLCONTENT`: render DirectComposition content (Windows 8.1+).
const PW_RENDERFULLCONTENT: PRINT_WINDOW_FLAGS = PRINT_WINDOW_FLAGS(2);

/// A PNG-encoded window capture.
#[derive(Clone, PartialEq, Eq)]
pub struct Screenshot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl Screenshot {
    pub fn base64(&self) -> String {
        to_base64(&self.png)
    }
}

impl std::fmt::Debug for Screenshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Screenshot")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("png_bytes", &self.png.len())
            .finish()
    }
}

/// Why no screenshot was produced, when that is not an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoScreenshot {
    Minimized,
    /// The capture was entirely black: protected video, or an app that
    /// does not support capture.
    Blank,
}

/// Captures the window's visible frame, downscaled so its longer edge is at
/// most `max_edge` pixels.
pub fn capture_window(
    id: WindowId,
    max_edge: u32,
) -> Result<std::result::Result<Screenshot, NoScreenshot>> {
    let h = live(id)?;
    // SAFETY: no memory preconditions.
    if unsafe { IsIconic(h) }.as_bool() {
        return Ok(Err(NoScreenshot::Minimized));
    }
    let outer = window_rect(h)?;
    let frame = frame_rect(h)?;
    let (w, ht) = (outer.width(), outer.height());
    if w <= 0 || ht <= 0 {
        return Ok(Err(NoScreenshot::Blank));
    }

    let bgra = render(h, w, ht)?;
    let full = bgra_to_rgb(&bgra, w as u32, ht as u32);
    // PrintWindow renders the outer rect, including invisible resize
    // borders. Crop to the visible frame.
    let img = crop(
        &full,
        (frame.left - outer.left).max(0) as u32,
        (frame.top - outer.top).max(0) as u32,
        frame.width().max(1) as u32,
        frame.height().max(1) as u32,
    );
    if is_blank(&img) {
        return Ok(Err(NoScreenshot::Blank));
    }
    let small = downscale(&img, max_edge);
    let png = encode_png(&small).map_err(|e| WinError::Api {
        op: "encode PNG",
        source: windows::core::Error::new(windows::core::HRESULT(-1), e),
    })?;
    Ok(Ok(Screenshot {
        png,
        width: small.width,
        height: small.height,
    }))
}

/// Renders the window into a top-down 32-bit BGRA buffer of `w` x `h`.
fn render(hwnd: windows::Win32::Foundation::HWND, w: i32, h: i32) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; w as usize * h as usize * 4];
    // SAFETY: every GDI object created here is selected out and released on
    // all paths before returning. `buf` holds exactly w*h 32-bit pixels, the
    // size GetDIBits writes for this BITMAPINFO (negative height = top-down).
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let previous = SelectObject(mem, HGDIOBJ(bitmap.0));

        let printed = PrintWindow(hwnd, mem, PW_RENDERFULLCONTENT).as_bool();

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let lines = if printed {
            GetDIBits(
                mem,
                bitmap,
                0,
                h as u32,
                Some(buf.as_mut_ptr().cast::<c_void>()),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(mem, previous);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        if !printed || lines == 0 {
            return Err(WinError::Api {
                op: "PrintWindow",
                source: windows::core::Error::from_thread(),
            });
        }
    }
    Ok(buf)
}

fn crop(img: &RgbImage, x: u32, y: u32, w: u32, h: u32) -> RgbImage {
    let w = w.min(img.width.saturating_sub(x));
    let h = h.min(img.height.saturating_sub(y));
    let mut data = Vec::with_capacity(w as usize * h as usize * 3);
    for row in y..y + h {
        let start = ((row * img.width + x) * 3) as usize;
        data.extend_from_slice(&img.data[start..start + w as usize * 3]);
    }
    RgbImage {
        width: w,
        height: h,
        data,
    }
}
