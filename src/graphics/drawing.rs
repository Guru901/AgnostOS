use crate::color::Color;

use super::{Framebuffer, PixelCoord, PixelRadius, PixelRows, PixelSize};

pub fn clear_background(fb: &Framebuffer, color: &Color) {
    if !fb.is_drawable() {
        return;
    }
    for row in 0..fb.height() {
        for col in 0..fb.width() {
            fb.write_pixel(row * fb.stride() + col, color);
        }
    }
}

pub fn draw_rec(fb: &Framebuffer, origin: PixelCoord, size: PixelSize, color: Color) {
    if !fb.is_drawable() {
        return;
    }
    let (x, y) = (origin.x(), origin.y());
    let (Some(x2), Some(y2)) = (x.checked_add(size.width()), y.checked_add(size.height())) else {
        return;
    };
    if x2 > fb.width() || y2 > fb.height() {
        return;
    }
    for row in y..y2 {
        for col in x..x2 {
            fb.write_pixel(row * fb.stride() + col, &color);
        }
    }
}

pub fn draw_circle(fb: &Framebuffer, radius: PixelRadius, center: PixelCoord, color: Color) {
    if !fb.is_drawable() {
        return;
    }
    let (Ok(radius), Ok(center_x), Ok(center_y), Ok(width), Ok(height)) = (
        isize::try_from(radius.get()),
        isize::try_from(center.x()),
        isize::try_from(center.y()),
        isize::try_from(fb.width()),
        isize::try_from(fb.height()),
    ) else {
        return;
    };
    let Some(radius_squared) = radius.checked_mul(radius) else {
        return;
    };
    for delta_y in -radius..=radius {
        for delta_x in -radius..=radius {
            let Some(distance_squared) = delta_x
                .checked_mul(delta_x)
                .and_then(|x| delta_y.checked_mul(delta_y)?.checked_add(x))
            else {
                continue;
            };
            let (Some(x), Some(y)) = (center_x.checked_add(delta_x), center_y.checked_add(delta_y))
            else {
                continue;
            };
            if distance_squared <= radius_squared && x >= 0 && y >= 0 && x < width && y < height {
                fb.write_pixel(y.cast_unsigned() * fb.stride() + x.cast_unsigned(), &color);
            }
        }
    }
}

pub fn draw_line(fb: &Framebuffer, start: PixelCoord, end: PixelCoord, color: Color) {
    if !fb.is_drawable() {
        return;
    }
    let (Ok(width), Ok(height), Ok(x2), Ok(y2), Ok(mut x), Ok(mut y)) = (
        i64::try_from(fb.width()),
        i64::try_from(fb.height()),
        i64::try_from(end.x()),
        i64::try_from(end.y()),
        i64::try_from(start.x()),
        i64::try_from(start.y()),
    ) else {
        return;
    };
    let (Ok(delta_x), Ok(delta_y)) = (i64::try_from(x.abs_diff(x2)), i64::try_from(y.abs_diff(y2)))
    else {
        return;
    };
    let step_x = if x < x2 { 1 } else { -1 };
    let step_y = if y < y2 { 1 } else { -1 };
    let mut error = delta_x - delta_y;
    loop {
        if x >= 0 && y >= 0 && x < width && y < height {
            let (Ok(row), Ok(column)) = (usize::try_from(y), usize::try_from(x)) else {
                return;
            };
            fb.write_pixel(row * fb.stride() + column, &color);
        }
        if x == x2 && y == y2 {
            break;
        }
        let Some(doubled_error) = error.checked_mul(2) else {
            return;
        };
        if doubled_error > -delta_y {
            let (Some(next_error), Some(next_x)) =
                (error.checked_sub(delta_y), x.checked_add(step_x))
            else {
                return;
            };
            error = next_error;
            x = next_x;
        }
        if doubled_error < delta_x {
            let (Some(next_error), Some(next_y)) =
                (error.checked_add(delta_x), y.checked_add(step_y))
            else {
                return;
            };
            error = next_error;
            y = next_y;
        }
    }
}

pub fn scroll_up(fb: &Framebuffer, rows: PixelRows) {
    fb.scroll_rows(rows.get());
}
