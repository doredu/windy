pub fn clamp_popup_position(
    cursor: (i32, i32),
    popup_size: (i32, i32),
    screen: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (cx, cy) = cursor;
    let (pw, ph) = popup_size;
    let (sx, sy, sw, sh) = screen;

    let mut x = cx;
    let mut y = cy;

    if x + pw > sx + sw {
        x = cx - pw;
    }
    if y + ph > sy + sh {
        y = cy - ph;
    }

    x = x.max(sx);
    y = y.max(sy);

    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_as_is_when_room_available() {
        let pos = clamp_popup_position((100, 100), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (100, 100));
    }

    #[test]
    fn flips_left_when_overflowing_right_edge() {
        let pos = clamp_popup_position((1850, 100), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (1650, 100));
    }

    #[test]
    fn flips_up_when_overflowing_bottom_edge() {
        let pos = clamp_popup_position((100, 1000), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (100, 700));
    }

    #[test]
    fn clamps_to_screen_origin_in_top_left_corner() {
        let pos = clamp_popup_position((-50, -50), (200, 300), (0, 0, 1920, 1080));
        assert_eq!(pos, (0, 0));
    }

    #[test]
    fn works_on_a_non_primary_monitor_with_negative_origin() {
        // secondary monitor to the left of the primary, virtual-screen coords are negative
        let pos = clamp_popup_position((-1800, 100), (200, 300), (-1920, 0, 1920, 1080));
        assert_eq!(pos, (-1800, 100));
    }
}
