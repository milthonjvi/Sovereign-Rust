//! Allocation-free software rasterization primitives.
//!
//! Triangle filling uses half-open scanline edge rules and emits horizontal
//! spans, avoiding per-pixel framebuffer API overhead. Coordinates are signed
//! so geometry can be positioned partially or wholly outside the target.
use super::color::Color;
use super::info::FramebufferError;
use super::surface::FramebufferSurface;


/// One exact scanline intersection. The private representation keeps the
/// integer floor and fractional remainder paired during sorting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScanlineIntersection {
    x_floor: i128,
    fractional: bool,
}

impl ScanlineIntersection {
    #[inline]
    fn is_left_of(self, other: Self) -> bool {
        self.x_floor < other.x_floor
            || (self.x_floor == other.x_floor && !self.fractional && other.fractional)
    }
}

/// Fill a polygon using the even-odd rule.
///
/// Vertices must contain at least three points. Scratch is caller-owned storage
/// and must hold at least vertices.len() intersections; no heap allocation is
/// performed. Horizontal edges are ignored and other edges use half-open
/// vertical coverage. The scratch requirement is checked before framebuffer
/// writes occur.
pub fn fill_polygon(
    target: &mut FramebufferSurface<'_>,
    vertices: &[(i64, i64)],
    color: Color,
    scratch: &mut [ScanlineIntersection],
) -> Result<(), FramebufferError> {
    if vertices.len() < 3 {
        return Ok(());
    }
    if scratch.len() < vertices.len() {
        return Err(FramebufferError::BufferTooSmall);
    }

    let info = *target.info();
    if info.width == 0 || info.height == 0 {
        return Ok(());
    }
    let mut min_y = i64::MAX;
    let mut max_y = i64::MIN;
    let mut min_x = i64::MAX;
    let mut max_x = i64::MIN;
    for &(x, y) in vertices {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let first_y = min_y.max(0);
    let last_y = max_y.min(info.height.saturating_sub(1) as i64);
    let clip_left = min_x.max(0) as i128;
    let clip_right = max_x.min(info.width.saturating_sub(1) as i64) as i128;
    if first_y > last_y || clip_left > clip_right {
        return Ok(());
    }

    for y in first_y..=last_y {
        let mut count = 0usize;
        for index in 0..vertices.len() {
            let start = vertices[index];
            let end = vertices[(index + 1) % vertices.len()];
            if start.1 == end.1 {
                continue;
            }
            let (lower, upper) = if start.1 < end.1 { (start, end) } else { (end, start) };
            if y < lower.1 || y >= upper.1 {
                continue;
            }

            let dx = (upper.0 as i128).checked_sub(lower.0 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let dy = (upper.1 as i128).checked_sub(lower.1 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let offset = (y as i128).checked_sub(lower.1 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let numerator = offset.checked_mul(dx)
                .ok_or(FramebufferError::SizeOverflow)?;
            scratch[count] = ScanlineIntersection {
                x_floor: (lower.0 as i128).checked_add(numerator.div_euclid(dy))
                    .ok_or(FramebufferError::SizeOverflow)?,
                fractional: numerator.rem_euclid(dy) != 0,
            };
            count += 1;
        }

        // Insertion sort suits small UI polygons and requires no allocator.
        for i in 1..count {
            let value = scratch[i];
            let mut j = i;
            while j > 0 && value.is_left_of(scratch[j - 1]) {
                scratch[j] = scratch[j - 1];
                j -= 1;
            }
            scratch[j] = value;
        }

        let mut pair = 0usize;
        while pair + 1 < count {
            let left_intersection = scratch[pair];
            let right_intersection = scratch[pair + 1];
            let left = (left_intersection.x_floor
                + if left_intersection.fractional { 1 } else { 0 }).max(clip_left);
            let right = right_intersection.x_floor.min(clip_right);
            if left <= right {
                let x = usize::try_from(left).map_err(|_| FramebufferError::SizeOverflow)?;
                let end = usize::try_from(right).map_err(|_| FramebufferError::SizeOverflow)?;
                target.fill_rect(x, y as usize, end - x + 1, 1, color);
            }
            pair += 2;
        }
    }
    Ok(())
}

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
    let mut fractional = [false; 3];

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

            let dx = (upper.0 as i128).checked_sub(lower.0 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let dy = (upper.1 as i128).checked_sub(lower.1 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let scan_offset = (y as i128).checked_sub(lower.1 as i128)
                .ok_or(FramebufferError::SizeOverflow)?;
            let numerator = scan_offset.checked_mul(dx)
                .ok_or(FramebufferError::SizeOverflow)?;
            let quotient = numerator.div_euclid(dy);
            let intersection = (lower.0 as i128)
                .checked_add(quotient)
                .ok_or(FramebufferError::SizeOverflow)?;
            intersections[count] = intersection;
            fractional[count] = numerator.rem_euclid(dy) != 0;
            count += 1;
        }

        if count < 2 {
            continue;
        }
        if intersections[0] > intersections[1] {
            intersections.swap(0, 1);
            fractional.swap(0, 1);
        }
        if count == 3 && intersections[1] > intersections[2] {
            intersections.swap(1, 2);
            fractional.swap(1, 2);
            if intersections[0] > intersections[1] {
                intersections.swap(0, 1);
                fractional.swap(0, 1);
            }
        }

        // Pixel centers are integer sample positions: ceil the left edge and
        // floor the right edge so fractional intersections never overdraw.
        let left_edge = intersections[0] + if fractional[0] { 1 } else { 0 };
        let left = left_edge.max(min_x as i128);
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
    target.draw_line(a.0, a.1, b.0, b.1, color);
    target.draw_line(b.0, b.1, c.0, c.1, color);
    target.draw_line(c.0, c.1, a.0, a.1, color);
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
    fn fills_concave_polygon_with_even_odd_rule() {
        use super::{fill_polygon, ScanlineIntersection};

        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        let vertices = [(1, 1), (6, 1), (6, 3), (3, 3), (3, 6), (1, 6)];
        let mut scratch = [ScanlineIntersection::default(); 6];
        fill_polygon(&mut target, &vertices, Color::RED, &mut scratch).unwrap();
        drop(target);

        let pixel = |x: usize, y: usize| memory[(y * 8 + x) * 4] == 255;
        assert!(pixel(5, 2));
        assert!(pixel(2, 5));
        assert!(!pixel(5, 5));
    }

    #[test]
    fn polygon_requires_sufficient_scratch_before_writing() {
        use super::{fill_polygon, ScanlineIntersection};

        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        let vertices = [(1, 1), (6, 1), (3, 6)];
        let mut scratch = [ScanlineIntersection::default(); 2];
        assert_eq!(
            fill_polygon(&mut target, &vertices, Color::RED, &mut scratch),
            Err(crate::drivers::framebuffer::info::FramebufferError::BufferTooSmall),
        );
        drop(target);
        assert!(memory.iter().all(|byte| *byte == 0));
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
    fn fractional_edges_do_not_overdraw_scanline_pixels() {
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        fill_triangle(
            &mut target,
            (1, 1),
            (6, 1),
            (3, 6),
            Color::RED,
        ).unwrap();
        drop(target);

        let pixel = |x: usize, y: usize| {
            let offset = (y * 8 + x) * 4;
            memory[offset] == 255
        };
        assert!(!pixel(1, 2));
        assert!(pixel(2, 2));
        assert!(pixel(5, 2));
        assert!(!pixel(6, 2));
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
