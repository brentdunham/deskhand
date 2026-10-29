//! Platform-independent rectangle math for window placement.

use serde::{Deserialize, Serialize};

/// A rectangle in physical screen pixels. `right` and `bottom` are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(&self) -> i32 {
        self.right - self.left
    }

    pub const fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub const fn contains(&self, other: &Rect) -> bool {
        other.left >= self.left
            && other.top >= self.top
            && other.right <= self.right
            && other.bottom <= self.bottom
    }
}

/// Where to place a window within a monitor's work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapTarget {
    LeftHalf,
    RightHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Two thirds of the work area in each dimension, centered.
    Center,
    /// The native maximized state, not just a full-size rectangle.
    Maximize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowState {
    Normal,
    Minimized,
    Maximized,
}

/// A window's show state and restored (normal) position. This is the undo data
/// for every placement change, and what named layouts store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub state: ShowState,
    /// Restored position in workspace coordinates, as `GetWindowPlacement`
    /// reports it. Only meaningful when passed back to `restore_placement`.
    pub normal: Rect,
}

/// The visible frame for `target` within `work`. For [`SnapTarget::Maximize`]
/// this is the whole work area; callers should use the native maximize state.
///
/// Halves split at `work.width() / 2`, so with an odd width the right side gets
/// the extra pixel and the two halves always tile the work area exactly.
pub fn snap_rect(work: Rect, target: SnapTarget) -> Rect {
    let mid_x = work.left + work.width() / 2;
    let mid_y = work.top + work.height() / 2;
    match target {
        SnapTarget::LeftHalf => Rect::new(work.left, work.top, mid_x, work.bottom),
        SnapTarget::RightHalf => Rect::new(mid_x, work.top, work.right, work.bottom),
        SnapTarget::TopHalf => Rect::new(work.left, work.top, work.right, mid_y),
        SnapTarget::BottomHalf => Rect::new(work.left, mid_y, work.right, work.bottom),
        SnapTarget::TopLeft => Rect::new(work.left, work.top, mid_x, mid_y),
        SnapTarget::TopRight => Rect::new(mid_x, work.top, work.right, mid_y),
        SnapTarget::BottomLeft => Rect::new(work.left, mid_y, mid_x, work.bottom),
        SnapTarget::BottomRight => Rect::new(mid_x, mid_y, work.right, work.bottom),
        SnapTarget::Center => {
            let w = work.width() * 2 / 3;
            let h = work.height() * 2 / 3;
            let left = work.left + (work.width() - w) / 2;
            let top = work.top + (work.height() - h) / 2;
            Rect::new(left, top, left + w, top + h)
        }
        SnapTarget::Maximize => work,
    }
}

/// Converts a desired visible frame into the outer rectangle to pass to
/// `SetWindowPos`.
///
/// On Windows 10 and 11, `GetWindowRect` includes invisible resize borders that
/// DWM does not draw. `window` is that outer rect and `frame` is the visible
/// frame (`DWMWA_EXTENDED_FRAME_BOUNDS`), both measured for the same window in
/// its current state. The difference between them is added back so the
/// visible edge lands exactly on `desired`.
pub fn outer_rect_for_frame(desired: Rect, window: Rect, frame: Rect) -> Rect {
    Rect::new(
        desired.left - (frame.left - window.left),
        desired.top - (frame.top - window.top),
        desired.right + (window.right - frame.right),
        desired.bottom + (window.bottom - frame.bottom),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1920x1080 monitor with a 48px taskbar at the bottom.
    const WORK: Rect = Rect::new(0, 0, 1920, 1032);

    #[test]
    fn halves_tile_the_work_area() {
        let l = snap_rect(WORK, SnapTarget::LeftHalf);
        let r = snap_rect(WORK, SnapTarget::RightHalf);
        assert_eq!(l, Rect::new(0, 0, 960, 1032));
        assert_eq!(r, Rect::new(960, 0, 1920, 1032));
        assert_eq!(l.width() + r.width(), WORK.width());

        let t = snap_rect(WORK, SnapTarget::TopHalf);
        let b = snap_rect(WORK, SnapTarget::BottomHalf);
        assert_eq!(t.bottom, b.top);
        assert_eq!(t.height() + b.height(), WORK.height());
    }

    #[test]
    fn odd_width_gives_extra_pixel_to_right_half() {
        let work = Rect::new(0, 0, 1921, 1000);
        let l = snap_rect(work, SnapTarget::LeftHalf);
        let r = snap_rect(work, SnapTarget::RightHalf);
        assert_eq!(l.width(), 960);
        assert_eq!(r.width(), 961);
        assert_eq!(l.right, r.left);
    }

    #[test]
    fn quarters_tile_the_work_area() {
        let quarters = [
            SnapTarget::TopLeft,
            SnapTarget::TopRight,
            SnapTarget::BottomLeft,
            SnapTarget::BottomRight,
        ]
        .map(|t| snap_rect(WORK, t));
        let area: i32 = quarters.iter().map(|q| q.width() * q.height()).sum();
        assert_eq!(area, WORK.width() * WORK.height());
        assert!(quarters.iter().all(|q| WORK.contains(q)));
    }

    #[test]
    fn respects_offset_work_area_on_secondary_monitor() {
        // Monitor to the left of the primary, taskbar on its left edge.
        let work = Rect::new(-2560 + 64, 0, 0, 1440);
        let l = snap_rect(work, SnapTarget::LeftHalf);
        assert_eq!(l.left, work.left);
        assert!(work.contains(&l));
    }

    #[test]
    fn center_is_two_thirds_and_centered() {
        let c = snap_rect(WORK, SnapTarget::Center);
        assert_eq!(c.width(), 1280);
        assert_eq!(c.height(), 688);
        assert_eq!(c.left - WORK.left, WORK.right - c.right);
        assert!(WORK.contains(&c));
    }

    #[test]
    fn outer_rect_adds_back_invisible_borders() {
        // Typical Windows 11 window: 7px invisible borders left, right, bottom.
        let window = Rect::new(100, 100, 914, 707);
        let frame = Rect::new(107, 100, 907, 700);
        let desired = Rect::new(0, 0, 960, 1032);
        let outer = outer_rect_for_frame(desired, window, frame);
        assert_eq!(outer, Rect::new(-7, 0, 967, 1039));
    }

    #[test]
    fn outer_rect_is_identity_without_borders() {
        let r = Rect::new(10, 20, 30, 40);
        assert_eq!(outer_rect_for_frame(WORK, r, r), WORK);
    }
}
