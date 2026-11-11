//! Allocation-free software rasterization primitives.
//!
//! Triangle filling uses half-open scanline edge rules and emits horizontal
//! spans, avoiding per-pixel framebuffer API overhead. Coordinates are signed
//! so geometry can be positioned partially or wholly outside the target.
use super::color::Color;
use super::info::FramebufferError;
use super::surface::FramebufferSurface;

/// Fill a triangle using integer scanline intersections.
///
/// Horizontal edges are excluded from edge intersection generation, and each
/// non-horizontal edge uses a half-open vertical interval. This prevents shared
/// vertices from being counted twice when adjacent triangles are rasterized.
pub fn fill_triangle(
    target: &mut FramebufferSurface<'_>,
    a: (i64, i64),
    b: (i64, i64),
    c: (i64, i64),
    color: Color,
) -> Result<(), FramebufferError> {
    let info = *target.info();
    if info.width == 0 || info.height == 0 {
        return Ok(());
    }

    let min_y = a.1.min(b.1).min(c.1).max(0);
    let max_y = a.1.max(b.1).max(c.1).min(info.height.saturating_sub(1) as i64);
    if min_y > max_y {
        return Ok(());
    }

    let min_x = a.0.min(b.0).min(c.0).max(0);
    let max_x = a.0.max(b.0).max(c.0).min(info.width.saturating_sub(1) as i64);
    if min_x > max_x {
        return Ok(());
    }

    let edges = [(a, b), (b, c), (c, a)];
    let mut intersections = [0i128; 3];

    for y in min_y..=max_y {
        let mut count = 0usize;
        for (start, end) in edges {
            let (x0, y0) = start;
            let (x1, y1) = end;
            if y0 == y1 {
                continue;
            }
            let (lower, upper) = if y0 < y1 { (start, end) } else { (end, start) };
            if y < lower.1 || y >= upper.1 {
                continue;
            }

            let dx = (x1 as i128).checked_sub(x0 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let dy = (y1 as i128).checked_sub(y0 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let scan_offset = (y as i128).checked_sub(y0 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let numerator = scan_offset.checked_mul(dx)
                .ok_or(FramebufferError::SizeOverflow)?;
            let intersection = (x0 as i128)
                .checked_add(numerator / dy)
                .ok_or(FramebufferError::SizeOverflow)?;
            intersections[count] = intersection;
            count += 1;
        }

        if count < 2 {
            continue;
        }
        if intersections[0] > intersections[1] {
            intersections.swap(0, 1);
        }
        if count == 3 && intersections[1] > intersections[2] {
            intersections.swap(1, 2);
            if intersections[0] > intersections[1] {
                intersections.swap(0, 1);
            }
        }

        let left = intersections[0].max(min_x as i128);
        let right = intersections[count - 1].min(max_x as i128);
        if left > right {
            continue;
        }
        let left = usize::try_from(left).map_err(|_| FramebufferError::SizeOverflow)?;
        let right = usize::try_from(right).map_err(|_| FramebufferError::SizeOverflow)?;
        target.fill_rect(left, y as usize, right - left + 1, 1, color);
    }

    Ok(())
}

/// Draw the three edges of a triangle using the surface's clipped line routine.
pub fn stroke_triangle(
    target: &mut FramebufferSurface<'_>,
    a: (i64, i64),
    b: (i64, i64),
    c: (i64, i64),
    color: Color,
) -> Result<(), FramebufferError> {
    target.draw_line(a.0, a.1, b.0, b.1, color)?;
    target.draw_line(b.0, b.1, c.0, c.1, color)?;
    target.draw_line(c.0, c.1, a.0, a.1, color)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{fill_triangle, stroke_triangle};
    use crate::drivers::framebuffer::color::{Color, PixelFormat};
    use crate::drivers::framebuffer::info::FramebufferInfo;
    use crate::drivers::framebuffer::surface::FramebufferSurface;

    fn surface<'a>(memory: &'a mut [u8; 8 * 8 * 4]) -> FramebufferSurface<'a> {
        let info = FramebufferInfo {
            address: 1,
            width: 8,
            height: 8,
            pitch: 8 * 4,
            bits_per_pixel: 32,
            pixel_format: PixelFormat::Rgb,
        };
        // SAFETY: the test array is writable, correctly sized, and exclusively borrowed.
        unsafe { FramebufferSurface::from_mapped(memory.as_mut_ptr(), memory.len(), info).unwrap() }
    }

    #[test]
    fn fills_triangle_as_horizontal_spans() {
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        fill_triangle(
            &mut target,
            (1, 1),
            (6, 1),
            (1, 6),
            Color::RED,
        ).unwrap();
        drop(target);

        assert_eq!(memory[(1 * 8 + 1) * 4..(1 * 8 + 1) * 4 + 4], [255, 0, 0, 0]);
        assert_eq!(memory[(2 * 8 + 2) * 4..(2 * 8 + 2) * 4 + 4], [255, 0, 0, 0]);
        assert_eq!(memory[(6 * 8 + 6) * 4..(6 * 8 + 6) * 4 + 4], [0, 0, 0, 0]);
    }

    #[test]
    fn rejects_unrepresentable_intersection_arithmetic() {
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        assert_eq!(
            fill_triangle(
                &mut target,
                (i64::MIN, i64::MIN),
                (i64::MAX, i64::MIN),
                (i64::MIN, i64::MAX),
                Color::WHITE,
            ),
            Err(crate::drivers::framebuffer::info::FramebufferError::SizeOverflow),
        );
    }

    #[test]
    fn outlines_triangle_edges() {
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        stroke_triangle(
            &mut target,
            (1, 1),
            (5, 1),
            (1, 5),
            Color::GREEN,
        ).unwrap();
        drop(target);
        assert_eq!(memory[(1 * 8 + 1) * 4..(1 * 8 + 1) * 4 + 4], [0, 255, 0, 0]);
    }
}
