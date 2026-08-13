/// The `popup_position` setting: where the popup's anchor point is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupPositionMode {
    Cursor,
    WindowCenter,
    ScreenCenter,
    BottomRight,
}

impl PopupPositionMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "window_center" => Self::WindowCenter,
            "screen_center" => Self::ScreenCenter,
            "bottom_right" => Self::BottomRight,
            _ => Self::Cursor,
        }
    }
}

/// The `popup_pin` setting: whether the popup renders above ("top") or below
/// ("bottom") its anchor point.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PopupPin {
    Top,
    Bottom,
}

impl PopupPin {
    pub fn parse(value: &str) -> Self {
        match value {
            "top" => Self::Top,
            _ => Self::Bottom,
        }
    }
}

/// Computes the popup's anchor point (before pin/clamp are applied) for a
/// given position mode. `cursor` and `screen` are always available;
/// `foreground_window` (left, top, width, height) is only available when
/// `GetForegroundWindow`/`GetWindowRect` succeeded, and `WindowCenter` falls
/// back to `ScreenCenter` when it isn't.
fn anchor_point(
    mode: PopupPositionMode,
    cursor: (i32, i32),
    foreground_window: Option<(i32, i32, i32, i32)>,
    screen: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (sx, sy, sw, sh) = screen;
    match mode {
        PopupPositionMode::Cursor => cursor,
        PopupPositionMode::ScreenCenter => (sx + sw / 2, sy + sh / 2),
        PopupPositionMode::WindowCenter => match foreground_window {
            Some((wx, wy, ww, wh)) => (wx + ww / 2, wy + wh / 2),
            None => (sx + sw / 2, sy + sh / 2),
        },
        PopupPositionMode::BottomRight => (sx + sw, sy + sh),
    }
}

/// Places the popup relative to `mode`'s anchor point, oriented by `pin`
/// (above or below the anchor), then clamps the result so the popup always
/// stays fully on-screen. This is the general form of `clamp_popup_position`
/// (which remains the `Cursor`+`Bottom` special case, still used as the
/// exact-legacy fallback when no settings are available).
pub fn compute_popup_position(
    mode: PopupPositionMode,
    pin: PopupPin,
    cursor: (i32, i32),
    foreground_window: Option<(i32, i32, i32, i32)>,
    popup_size: (i32, i32),
    screen: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (ax, ay) = anchor_point(mode, cursor, foreground_window, screen);
    let (pw, ph) = popup_size;
    let (sx, sy, sw, sh) = screen;

    // Horizontally: left-align to the anchor for Cursor (matching legacy
    // behavior exactly), center on it for the two center modes, and
    // right-align for BottomRight so the popup sits inside that corner.
    let mut x = match mode {
        PopupPositionMode::Cursor => ax,
        PopupPositionMode::BottomRight => ax - pw,
        _ => ax - pw / 2,
    };
    let mut y = match pin {
        PopupPin::Bottom => ay,
        PopupPin::Top => ay - ph,
    };

    x = x.clamp(sx, (sx + sw - pw).max(sx));
    y = y.clamp(sy, (sy + sh - ph).max(sy));

    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor_bottom(cursor: (i32, i32), popup_size: (i32, i32), screen: (i32, i32, i32, i32)) -> (i32, i32) {
        compute_popup_position(PopupPositionMode::Cursor, PopupPin::Bottom, cursor, None, popup_size, screen)
    }

    #[test]
    fn fits_as_is_when_room_available() {
        let pos = cursor_bottom((100, 100), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (100, 100));
    }

    #[test]
    fn clamps_when_overflowing_right_edge() {
        let pos = cursor_bottom((1850, 100), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (1720, 100));
    }

    #[test]
    fn clamps_when_overflowing_bottom_edge() {
        let pos = cursor_bottom((100, 1000), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (100, 780));
    }

    #[test]
    fn clamps_to_screen_origin_in_top_left_corner() {
        let pos = cursor_bottom((-50, -50), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (0, 0));
    }

    #[test]
    fn works_on_a_non_primary_monitor_with_negative_origin() {
        // secondary monitor to the left of the primary, virtual-screen coords are negative
        let pos = cursor_bottom((-1800, 100), (200, 300), (-1920, 0, 1920, 1080));
        assert_eq!(pos, (-1800, 100));
    }

    #[test]
    fn cursor_mode_bottom_pin_matches_left_aligned_top_edge() {
        let pos = compute_popup_position(
            PopupPositionMode::Cursor,
            PopupPin::Bottom,
            (500, 500),
            None,
            (200, 300),
            (0, 0, 1920, 1080),
        );
        assert_eq!(pos, (500, 500));
    }

    #[test]
    fn cursor_mode_top_pin_places_popup_above_cursor() {
        let pos = compute_popup_position(
            PopupPositionMode::Cursor,
            PopupPin::Top,
            (500, 500),
            None,
            (200, 300),
            (0, 0, 1920, 1080),
        );
        assert_eq!(pos, (500, 200));
    }

    #[test]
    fn screen_center_mode_centers_popup_on_screen_midpoint() {
        let pos = compute_popup_position(
            PopupPositionMode::ScreenCenter,
            PopupPin::Bottom,
            (0, 0),
            None,
            (200, 300),
            (0, 0, 1920, 1080),
        );
        // screen midpoint is (960, 540); popup centers horizontally there
        assert_eq!(pos, (860, 540));
    }

    #[test]
    fn window_center_mode_uses_foreground_window_bounds() {
        let pos = compute_popup_position(
            PopupPositionMode::WindowCenter,
            PopupPin::Bottom,
            (0, 0),
            Some((100, 100, 800, 600)),
            (200, 300),
            (0, 0, 1920, 1080),
        );
        // window center is (500, 400)
        assert_eq!(pos, (400, 400));
    }

    #[test]
    fn window_center_mode_falls_back_to_screen_center_without_foreground_window() {
        let pos = compute_popup_position(
            PopupPositionMode::WindowCenter,
            PopupPin::Bottom,
            (0, 0),
            None,
            (200, 300),
            (0, 0, 1920, 1080),
        );
        assert_eq!(pos, (860, 540));
    }

    #[test]
    fn bottom_right_mode_right_aligns_into_the_corner() {
        let pos = compute_popup_position(
            PopupPositionMode::BottomRight,
            PopupPin::Top,
            (0, 0),
            None,
            (200, 300),
            (0, 0, 1920, 1080),
        );
        assert_eq!(pos, (1720, 780));
    }

    #[test]
    fn result_is_always_clamped_on_screen() {
        let pos = compute_popup_position(
            PopupPositionMode::Cursor,
            PopupPin::Bottom,
            (-9999, -9999),
            None,
            (200, 300),
            (0, 0, 1920, 1080),
        );
        assert_eq!(pos, (0, 0));
    }
}
