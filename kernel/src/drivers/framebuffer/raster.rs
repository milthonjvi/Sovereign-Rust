//! Allocation-free software rasterization primitives.
//!
//! Triangle filling uses half-open scanline edge rules and emits horizontal
//! spans, avoiding per-pixel framebuffer API overhead. Coordinates are signed
//! so geometry can be positioned partially or wholly outside the target.
use super::color::Color;
use super::info::FramebufferError;
use super::surface::FramebufferSurface;


/// Preflight the greatest visible scanline offset for an edge. Since the
/// intersection numerator is linear in the non-negative offset, checking its
/// maximum magnitude before rasterization prevents arithmetic errors after
/// earlier rows have already been modified.
fn validate_edge_range(
    lower: (i64, i64),
    upper: (i64, i64),
    first_y: i64,
    last_y: i64,
) -> Result<(), FramebufferError> {
    if lower.1 == upper.1 || first_y > last_y {
        return Ok(());
    }
    let start_y = first_y.max(lower.1);
    let end_y = last_y.min(upper.1.saturating_sub(1));
    if start_y > end_y {
        return Ok(());
    }
    let dx = (upper.0 as i128).checked_sub(lower.0 as i128)
        .ok_or(FramebufferError::SizeOverflow)?;
    let dy = (upper.1 as i128).checked_sub(lower.1 as i128)
        .ok_or(FramebufferError::SizeOverflow)?;
    let offset = (end_y as i128).checked_sub(lower.1 as i128)
        .ok_or(FramebufferError::SizeOverflow)?;
    let numerator = offset.checked_mul(dx)
        .ok_or(FramebufferError::SizeOverflow)?;
    (lower.0 as i128).checked_add(numerator.div_euclid(dy))
        .ok_or(FramebufferError::SizeOverflow)?;
    Ok(())
}

/// One exact scanline intersection. The private representation keeps the
/// integer floor and fractional remainder paired during sorting.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct ScanlineIntersection {
    x_floor: i128,
    fractional: bool,
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

    // Validate every edge before the first write so malformed/extreme geometry
    // cannot leave a partially rendered polygon on arithmetic failure.
    for index in 0..vertices.len() {
        let start = vertices[index];
        let end = vertices[(index + 1) % vertices.len()];
        let (lower, upper) = if start.1 <= end.1 { (start, end) } else { (end, start) };
        validate_edge_range(lower, upper, first_y, last_y)?;
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

        // Core's unstable slice sort is allocation-free and scales better than
        // insertion sort for complex paths with many active edges.
        scratch[..count].sort_unstable();

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


/// Stroke a closed polygon by connecting each vertex to the next.
///
/// Coordinates are signed and clipping is delegated to the surface line
/// primitive. Fewer than two vertices produce no output.
pub fn stroke_polygon(
    target: &mut FramebufferSurface<'_>,
    vertices: &[(i64, i64)],
    color: Color,
) {
    if vertices.len() < 2 {
        return;
    }
    for index in 0..vertices.len() {
        let start = vertices[index];
        let end = vertices[(index + 1) % vertices.len()];
        target.draw_line(start.0, start.1, end.0, end.1, color);
    }
}

/// Draw an antialiased line using fixed-point coverage interpolation.
///
/// The major axis is clipped before iteration, so work is bounded by the
/// framebuffer dimensions even for endpoints near the limits of `i64`.
/// Coverage is composited through the surface's RGB/BGR-aware alpha path.
pub fn stroke_line_aa(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    end: (i64, i64),
    color: Color,
) -> Result<(), FramebufferError> {
    let info = *target.info();
    if info.width == 0 || info.height == 0 || color.alpha == 0 {
        return Ok(());
    }

    let dx = (end.0 as i128 - start.0 as i128).unsigned_abs();
    let dy = (end.1 as i128 - start.1 as i128).unsigned_abs();
    if dx == 0 && dy == 0 {
        if start.0 >= 0 && start.1 >= 0
            && (start.0 as i128) < info.width as i128
            && (start.1 as i128) < info.height as i128
        {
            target.blend_pixel(start.0 as usize, start.1 as usize, color)?;
        }
        return Ok(());
    }

    let steep = dy > dx;
    let (mut x0, mut y0, mut x1, mut y1) = if steep {
        (start.1, start.0, end.1, end.0)
    } else {
        (start.0, start.1, end.0, end.1)
    };
    if x0 > x1 {
        core::mem::swap(&mut x0, &mut x1);
        core::mem::swap(&mut y0, &mut y1);
    }

    let major_limit = if steep { info.height } else { info.width };
    let minor_limit = if steep { info.width } else { info.height };
    if major_limit == 0 || minor_limit == 0 {
        return Ok(());
    }
    let major_delta = (x1 as i128 - x0 as i128) as u128;
    let minor_delta = (y1 as i128 - y0 as i128).unsigned_abs();
    let direction = if y1 >= y0 { 1i128 } else { -1i128 };
    let first = x0.max(0);
    let last = x1.min(i64::try_from(major_limit - 1).unwrap_or(i64::MAX));
    if first > last {
        return Ok(());
    }

    for major in first..=last {
        let distance = (major as i128 - x0 as i128) as u128;
        // distance and minor_delta are each at most 2^64-1, so this
        // multiplication fits in u128. Split quotient/remainder before
        // converting to 16.16 fixed point to avoid a 2^144 intermediate.
        let product = distance * minor_delta;
        let whole = product / major_delta;
        let remainder = product % major_delta;
        let fraction = (remainder * 65_536 + major_delta / 2) / major_delta;
        let offset = whole * 65_536 + fraction;
        let minor_fixed = y0 as i128 * 65_536 + direction * offset as i128;
        let minor_floor = minor_fixed.div_euclid(65_536);
        let minor_fraction = minor_fixed.rem_euclid(65_536) as u32;

        let primary_coverage = 65_536 - minor_fraction;
        let primary_alpha = ((color.alpha as u32 * primary_coverage + 32_768) / 65_536) as u8;
        if minor_floor >= 0 && minor_floor < minor_limit as i128 && primary_alpha != 0 {
            let (x, y) = if steep {
                (minor_floor as usize, major as usize)
            } else {
                (major as usize, minor_floor as usize)
            };
            target.blend_pixel(x, y, Color::with_alpha(color.red, color.green, color.blue, primary_alpha))?;
        }

        if minor_fraction != 0 {
            let secondary = minor_floor + 1;
            let secondary_alpha = ((color.alpha as u32 * minor_fraction + 32_768) / 65_536) as u8;
            if secondary >= 0 && secondary < minor_limit as i128 && secondary_alpha != 0 {
                let (x, y) = if steep {
                    (secondary as usize, major as usize)
                } else {
                    (major as usize, secondary as usize)
                };
                target.blend_pixel(x, y, Color::with_alpha(color.red, color.green, color.blue, secondary_alpha))?;
            }
        }
    }
    Ok(())
}

/// Stroke an open polyline by connecting consecutive vertices.
///
/// Unlike `stroke_polygon`, this function does not connect the last vertex
/// back to the first. The caller owns the vertex slice; no allocation occurs.
pub fn stroke_polyline(
    target: &mut FramebufferSurface<'_>,
    vertices: &[(i64, i64)],
    color: Color,
) {
    for segment in vertices.windows(2) {
        let start = segment[0];
        let end = segment[1];
        target.draw_line(start.0, start.1, end.0, end.1, color);
    }
}

/// Draw an open polyline with antialiased segment coverage.
pub fn stroke_polyline_aa(
    target: &mut FramebufferSurface<'_>,
    vertices: &[(i64, i64)],
    color: Color,
) -> Result<(), FramebufferError> {
    for segment in vertices.windows(2) {
        stroke_line_aa(target, segment[0], segment[1], color)?;
    }
    Ok(())
}

/// Fill an axis-aligned ellipse using integer scan conversion.
///
/// Radii larger than i64::MAX are rejected before any framebuffer writes.
/// The scan is clipped vertically and each emitted horizontal span is clipped
/// to the target, so work is bounded by the visible framebuffer height.
pub fn fill_ellipse(
    target: &mut FramebufferSurface<'_>,
    center_x: i64,
    center_y: i64,
    radius_x: usize,
    radius_y: usize,
    color: Color,
) -> Result<(), FramebufferError> {
    if radius_x > i64::MAX as usize || radius_y > i64::MAX as usize {
        return Err(FramebufferError::SizeOverflow);
    }
    let info = *target.info();
    if info.width == 0 || info.height == 0 { return Ok(()); }
    let rx = radius_x as u128;
    let ry = radius_y as u128;
    let cx = center_x as i128;
    let cy = center_y as i128;
    if radius_y == 0 {
        let left = (cx - rx as i128).max(0);
        let right = (cx + rx as i128).min(info.width.saturating_sub(1) as i128);
        if cy >= 0 && cy < info.height as i128 && left <= right {
            let x = usize::try_from(left).map_err(|_| FramebufferError::SizeOverflow)?;
            let width = usize::try_from(right - left + 1).map_err(|_| FramebufferError::SizeOverflow)?;
            target.fill_rect(x, cy as usize, width, 1, color);
        }
        return Ok(());
    }
    if radius_x == 0 {
        let first = (cy - ry as i128).max(0);
        let last = (cy + ry as i128).min(info.height.saturating_sub(1) as i128);
        if cx >= 0 && cx < info.width as i128 && first <= last {
            for y in first..=last { target.fill_rect(cx as usize, y as usize, 1, 1, color); }
        }
        return Ok(());
    }
    let first_y = (cy - ry as i128).max(0);
    let last_y = (cy + ry as i128).min(info.height.saturating_sub(1) as i128);
    if first_y > last_y { return Ok(()); }

    // Scan along the major radius. Keeping the computed span on the minor
    // axis avoids magnifying integer-square-root truncation for very wide
    // ellipses (where rx / ry could otherwise be arbitrarily large).
    if rx > ry {
        let first_x = (cx - rx as i128).max(0);
        let last_x = (cx + rx as i128).min(info.width.saturating_sub(1) as i128);
        if first_x > last_x { return Ok(()); }
        let rx_squared = rx * rx;
        for x in first_x..=last_x {
            let dx = (x - cx).unsigned_abs();
            if dx > rx { continue; }
            let vertical = (ry * integer_sqrt(rx_squared - dx * dx)) / rx;
            let top = (cy - vertical as i128).max(0);
            let bottom = (cy + vertical as i128).min(info.height.saturating_sub(1) as i128);
            if top <= bottom {
                let height = usize::try_from(bottom - top + 1)
                    .map_err(|_| FramebufferError::SizeOverflow)?;
                target.fill_rect(x as usize, top as usize, 1, height, color);
            }
        }
        return Ok(());
    }

    let ry_squared = ry * ry;
    for y in first_y..=last_y {
        let dy = (y - cy).unsigned_abs();
        if dy > ry { continue; }
        let remaining = ry_squared - dy * dy;
        let horizontal = (rx * integer_sqrt(remaining)) / ry;
        let left = (cx - horizontal as i128).max(0);
        let right = (cx + horizontal as i128).min(info.width.saturating_sub(1) as i128);
        if left <= right {
            let x = usize::try_from(left).map_err(|_| FramebufferError::SizeOverflow)?;
            let width = usize::try_from(right - left + 1).map_err(|_| FramebufferError::SizeOverflow)?;
            target.fill_rect(x, y as usize, width, 1, color);
        }
    }
    Ok(())
}

#[inline]
fn integer_sqrt(value: u128) -> u128 {
    let mut remainder = value;
    let mut result = 0u128;
    let mut bit = 1u128 << 126;
    while bit > remainder { bit >>= 2; }
    while bit != 0 {
        if remainder >= result + bit {
            remainder -= result + bit;
            result = (result >> 1) + bit;
        } else {
            result >>= 1;
        }
        bit >>= 2;
    }
    result
}
/// Draw an axis-aligned ellipse outline using integer scan conversion.
/// Each visible scanline contributes its left and right boundary pixels.
pub fn stroke_ellipse(
    target: &mut FramebufferSurface<'_>,
    center_x: i64,
    center_y: i64,
    radius_x: usize,
    radius_y: usize,
    color: Color,
) -> Result<(), FramebufferError> {
    if radius_x > i64::MAX as usize || radius_y > i64::MAX as usize {
        return Err(FramebufferError::SizeOverflow);
    }
    let info = *target.info();
    if info.width == 0 || info.height == 0 { return Ok(()); }
    let rx = radius_x as u128;
    let ry = radius_y as u128;
    let cx = center_x as i128;
    let cy = center_y as i128;
    if radius_y == 0 {
        return fill_ellipse(target, center_x, center_y, radius_x, 0, color);
    }
    let first_y = (cy - ry as i128).max(0);
    let last_y = (cy + ry as i128).min(info.height.saturating_sub(1) as i128);
    if first_y > last_y { return Ok(()); }

    // As in fill_ellipse, rasterize along the larger radius so integer sqrt
    // truncation affects only the smaller span, by less than one pixel.
    if rx > ry {
        let first_x = (cx - rx as i128).max(0);
        let last_x = (cx + rx as i128).min(info.width.saturating_sub(1) as i128);
        if first_x > last_x { return Ok(()); }
        let rx_squared = rx * rx;
        for x in first_x..=last_x {
            let dx = (x - cx).unsigned_abs();
            if dx > rx { continue; }
            let vertical = (ry * integer_sqrt(rx_squared - dx * dx)) / rx;
            let top = cy - vertical as i128;
            let bottom = cy + vertical as i128;
            if top >= 0 && top < info.height as i128 {
                target.put_pixel(x as usize, top as usize, color)?;
            }
            if bottom != top && bottom >= 0 && bottom < info.height as i128 {
                target.put_pixel(x as usize, bottom as usize, color)?;
            }
        }
        return Ok(());
    }

    let ry_squared = ry * ry;
    for y in first_y..=last_y {
        let dy = (y - cy).unsigned_abs();
        if dy > ry { continue; }
        let horizontal = (rx * integer_sqrt(ry_squared - dy * dy)) / ry;
        let left = cx - horizontal as i128;
        let right = cx + horizontal as i128;
        if left >= 0 && left < info.width as i128 {
            target.put_pixel(left as usize, y as usize, color)?;
        }
        if right != left && right >= 0 && right < info.width as i128 {
            target.put_pixel(right as usize, y as usize, color)?;
        }
    }
    Ok(())
}
/// Stroke a quadratic Bézier curve with bounded, allocation-free subdivision.
/// Depth is clamped to 10, limiting work to at most 1024 line segments.
pub fn stroke_quadratic_bezier(target: &mut FramebufferSurface<'_>, start: (i64, i64), control: (i64, i64), end: (i64, i64), subdivision_depth: u8, color: Color) {
    quadratic_subdivide(target, start, control, end, subdivision_depth.min(10), color);
}

#[inline]
fn midpoint(a: (i64, i64), b: (i64, i64)) -> (i64, i64) {
    (((a.0 as i128 + b.0 as i128) / 2) as i64, ((a.1 as i128 + b.1 as i128) / 2) as i64)
}

fn quadratic_subdivide(target: &mut FramebufferSurface<'_>, start: (i64, i64), control: (i64, i64), end: (i64, i64), depth: u8, color: Color) {
    if depth == 0 { target.draw_line(start.0, start.1, end.0, end.1, color); return; }
    let a = midpoint(start, control);
    let b = midpoint(control, end);
    let center = midpoint(a, b);
    quadratic_subdivide(target, start, a, center, depth - 1, color);
    quadratic_subdivide(target, center, b, end, depth - 1, color);
}

/// Stroke a cubic Bézier curve using de Casteljau subdivision.
/// Depth is clamped to 10, limiting work to at most 1024 line segments.
pub fn stroke_cubic_bezier(target: &mut FramebufferSurface<'_>, start: (i64, i64), control1: (i64, i64), control2: (i64, i64), end: (i64, i64), subdivision_depth: u8, color: Color) {
    cubic_subdivide(target, start, control1, control2, end, subdivision_depth.min(10), color);
}

fn cubic_subdivide(target: &mut FramebufferSurface<'_>, start: (i64, i64), control1: (i64, i64), control2: (i64, i64), end: (i64, i64), depth: u8, color: Color) {
    if depth == 0 { target.draw_line(start.0, start.1, end.0, end.1, color); return; }
    let a = midpoint(start, control1);
    let b = midpoint(control1, control2);
    let c = midpoint(control2, end);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let center = midpoint(d, e);
    cubic_subdivide(target, start, a, d, center, depth - 1, color);
    cubic_subdivide(target, center, e, c, end, depth - 1, color);
}
/// Stroke a quadratic Bézier curve with adaptive flatness subdivision.
///
/// `flatness_tolerance` is measured in framebuffer pixels using a conservative
/// L-infinity distance bound. Subdivision is allocation-free and bounded by
/// `max_depth` (clamped to 10). Arithmetic overflow in the flatness estimate
/// conservatively requests another subdivision rather than accepting a coarse
/// segment.
pub fn stroke_quadratic_bezier_adaptive(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control: (i64, i64),
    end: (i64, i64),
    flatness_tolerance: usize,
    max_depth: u8,
    color: Color,
) {
    let tolerance = flatness_tolerance.min(i64::MAX as usize) as u128;
    quadratic_adaptive(target, start, control, end, tolerance, max_depth.min(10), color);
}

fn quadratic_adaptive(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control: (i64, i64),
    end: (i64, i64),
    tolerance: u128,
    depth: u8,
    color: Color,
) {
    if depth == 0 || point_within_chord_tolerance(start, control, end, tolerance) {
        target.draw_line(start.0, start.1, end.0, end.1, color);
        return;
    }
    let a = midpoint(start, control);
    let b = midpoint(control, end);
    let center = midpoint(a, b);
    quadratic_adaptive(target, start, a, center, tolerance, depth - 1, color);
    quadratic_adaptive(target, center, b, end, tolerance, depth - 1, color);
}

/// Stroke a cubic Bézier curve with adaptive flatness subdivision.
///
/// Both control points must satisfy the chord-distance tolerance before a
/// segment is emitted. Work and recursion depth are bounded by `max_depth`,
/// clamped to 10; no temporary heap storage is used.
pub fn stroke_cubic_bezier_adaptive(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control1: (i64, i64),
    control2: (i64, i64),
    end: (i64, i64),
    flatness_tolerance: usize,
    max_depth: u8,
    color: Color,
) {
    let tolerance = flatness_tolerance.min(i64::MAX as usize) as u128;
    cubic_adaptive(
        target, start, control1, control2, end, tolerance, max_depth.min(10), color,
    );
}

fn cubic_adaptive(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control1: (i64, i64),
    control2: (i64, i64),
    end: (i64, i64),
    tolerance: u128,
    depth: u8,
    color: Color,
) {
    if depth == 0
        || (point_within_chord_tolerance(start, control1, end, tolerance)
            && point_within_chord_tolerance(start, control2, end, tolerance))
    {
        target.draw_line(start.0, start.1, end.0, end.1, color);
        return;
    }
    let a = midpoint(start, control1);
    let b = midpoint(control1, control2);
    let c = midpoint(control2, end);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let center = midpoint(d, e);
    cubic_adaptive(target, start, a, d, center, tolerance, depth - 1, color);
    cubic_adaptive(target, center, e, c, end, tolerance, depth - 1, color);
}

/// Stroke an axis-aligned ellipse with antialiased adaptive Bézier arcs.
///
/// The four cubic arcs approximate a mathematical ellipse using the standard
/// kappa constant (0.55228475). Coordinates that cannot be represented by the
/// signed raster API are rejected before any framebuffer writes.
pub fn stroke_ellipse_aa(
    target: &mut FramebufferSurface<'_>,
    center_x: i64,
    center_y: i64,
    radius_x: usize,
    radius_y: usize,
    color: Color,
) -> Result<(), FramebufferError> {
    if radius_x > i64::MAX as usize || radius_y > i64::MAX as usize {
        return Err(FramebufferError::SizeOverflow);
    }
    let rx = radius_x as i128;
    let ry = radius_y as i128;
    let cx = center_x as i128;
    let cy = center_y as i128;
    let offset_x = (rx * 36_195 + 32_768) / 65_536;
    let offset_y = (ry * 36_195 + 32_768) / 65_536;
    let point = |x: i128, y: i128| -> Result<(i64, i64), FramebufferError> {
        Ok((
            i64::try_from(x).map_err(|_| FramebufferError::SizeOverflow)?,
            i64::try_from(y).map_err(|_| FramebufferError::SizeOverflow)?,
        ))
    };

    if radius_x == 0 || radius_y == 0 {
        return stroke_line_aa(
            target,
            point(cx - rx, cy - ry)?,
            point(cx + rx, cy + ry)?,
            color,
        );
    }

    // Preflight all control points so overflow cannot leave a partial ellipse.
    let right = point(cx + rx, cy)?;
    let bottom = point(cx, cy + ry)?;
    let left = point(cx - rx, cy)?;
    let top = point(cx, cy - ry)?;
    let right_down = point(cx + rx, cy + offset_y)?;
    let down_right = point(cx + offset_x, cy + ry)?;
    let down_left = point(cx - offset_x, cy + ry)?;
    let left_down = point(cx - rx, cy + offset_y)?;
    let left_up = point(cx - rx, cy - offset_y)?;
    let up_left = point(cx - offset_x, cy - ry)?;
    let up_right = point(cx + offset_x, cy - ry)?;
    let right_up = point(cx + rx, cy - offset_y)?;

    stroke_cubic_bezier_aa(
        target, right, right_down, down_right, bottom, 1, 10, color,
    )?;
    stroke_cubic_bezier_aa(
        target, bottom, down_left, left_down, left, 1, 10, color,
    )?;
    stroke_cubic_bezier_aa(
        target, left, left_up, up_left, top, 1, 10, color,
    )?;
    stroke_cubic_bezier_aa(
        target, top, up_right, right_up, right, 1, 10, color,
    )
}

/// Stroke a quadratic Bézier curve with adaptive subdivision and antialiased segments.
pub fn stroke_quadratic_bezier_aa(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control: (i64, i64),
    end: (i64, i64),
    flatness_tolerance: usize,
    max_depth: u8,
    color: Color,
) -> Result<(), FramebufferError> {
    let tolerance = flatness_tolerance.min(i64::MAX as usize) as u128;
    quadratic_adaptive_aa(
        target, start, control, end, tolerance, max_depth.min(10), color,
    )
}

fn quadratic_adaptive_aa(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control: (i64, i64),
    end: (i64, i64),
    tolerance: u128,
    depth: u8,
    color: Color,
) -> Result<(), FramebufferError> {
    if depth == 0 || point_within_chord_tolerance(start, control, end, tolerance) {
        return stroke_line_aa(target, start, end, color);
    }
    let a = midpoint(start, control);
    let b = midpoint(control, end);
    let center = midpoint(a, b);
    quadratic_adaptive_aa(target, start, a, center, tolerance, depth - 1, color)?;
    quadratic_adaptive_aa(target, center, b, end, tolerance, depth - 1, color)
}

/// Stroke a cubic Bézier curve with adaptive subdivision and antialiased segments.
pub fn stroke_cubic_bezier_aa(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control1: (i64, i64),
    control2: (i64, i64),
    end: (i64, i64),
    flatness_tolerance: usize,
    max_depth: u8,
    color: Color,
) -> Result<(), FramebufferError> {
    let tolerance = flatness_tolerance.min(i64::MAX as usize) as u128;
    cubic_adaptive_aa(
        target, start, control1, control2, end, tolerance, max_depth.min(10), color,
    )
}

fn cubic_adaptive_aa(
    target: &mut FramebufferSurface<'_>,
    start: (i64, i64),
    control1: (i64, i64),
    control2: (i64, i64),
    end: (i64, i64),
    tolerance: u128,
    depth: u8,
    color: Color,
) -> Result<(), FramebufferError> {
    if depth == 0
        || (point_within_chord_tolerance(start, control1, end, tolerance)
            && point_within_chord_tolerance(start, control2, end, tolerance))
    {
        return stroke_line_aa(target, start, end, color);
    }
    let a = midpoint(start, control1);
    let b = midpoint(control1, control2);
    let c = midpoint(control2, end);
    let d = midpoint(a, b);
    let e = midpoint(b, c);
    let center = midpoint(d, e);
    cubic_adaptive_aa(target, start, a, d, center, tolerance, depth - 1, color)?;
    cubic_adaptive_aa(target, center, e, c, end, tolerance, depth - 1, color)
}

/// Conservative distance test using the chord's cross product. Checked i128
/// arithmetic avoids overflow for extreme signed coordinates; on overflow the
/// caller subdivides further. The zero-length chord uses an L-infinity test.
#[inline]
fn point_within_chord_tolerance(
    start: (i64, i64),
    point: (i64, i64),
    end: (i64, i64),
    tolerance: u128,
) -> bool {
    let dx = end.0 as i128 - start.0 as i128;
    let dy = end.1 as i128 - start.1 as i128;
    let px = point.0 as i128 - start.0 as i128;
    let py = point.1 as i128 - start.1 as i128;
    if dx == 0 && dy == 0 {
        return px.unsigned_abs().max(py.unsigned_abs()) <= tolerance;
    }
    let Some(cross) = dx.checked_mul(py)
        .and_then(|left| dy.checked_mul(px).and_then(|right| left.checked_sub(right)))
    else {
        return false;
    };
    let scale = dx.unsigned_abs().max(dy.unsigned_abs());
    // tolerance is capped at i64::MAX, so this product is strictly below
    // 2^127 even for the largest possible difference of two i64 coordinates.
    let margin = tolerance * scale;
    if cross.unsigned_abs() > margin {
        return false;
    }

    // Distance to the infinite chord alone is insufficient: a collinear
    // control point can lie beyond either endpoint and make the curve reverse
    // along the same line. Bound its projection to the finite segment too.
    let Some(projection) = dx.checked_mul(px)
        .and_then(|x| dy.checked_mul(py).and_then(|y| x.checked_add(y)))
    else {
        return false;
    };
    let Some(chord_length_squared) = dx.checked_mul(dx)
        .and_then(|x| dy.checked_mul(dy).and_then(|y| x.checked_add(y)))
    else {
        return false;
    };
    let margin_i128 = margin as i128;
    let Some(upper_projection) = chord_length_squared.checked_add(margin_i128) else {
        return false;
    };
    projection >= -margin_i128 && projection <= upper_projection
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
    for (start, end) in edges {
        let (lower, upper) = if start.1 <= end.1 { (start, end) } else { (end, start) };
        validate_edge_range(lower, upper, min_y, max_y)?;
    }
    let mut intersections = [0i128; 3];
    let mut fractional = [false; 3];

    for y in min_y..=max_y {
        let mut count = 0usize;
        for (start, end) in edges {
            let (_, y0) = start;
            let (_, y1) = end;
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
    use super::{fill_triangle, stroke_polygon, stroke_triangle};
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
    fn quadratic_bezier_includes_endpoints() {
        use super::stroke_quadratic_bezier;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        stroke_quadratic_bezier(&mut target, (1, 6), (4, 0), (6, 6), 3, Color::GREEN);
        drop(target);
        let pixel = |x: usize, y: usize| memory[(y * 8 + x) * 4 + 1] == 255;
        assert!(pixel(1, 6));
        assert!(pixel(6, 6));
        assert!(pixel(3, 3) || pixel(4, 3));
    }

    #[test]
    fn cubic_bezier_handles_extreme_control_points_without_overflow() {
        use super::stroke_cubic_bezier;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        stroke_cubic_bezier(&mut target, (1, 1), (i64::MIN, i64::MAX), (i64::MAX, i64::MIN), (6, 6), 255, Color::RED);
        drop(target);
        assert_eq!(memory[(1 * 8 + 1) * 4], 255);
        assert_eq!(memory[(6 * 8 + 6) * 4], 255);
    }
    #[test]
    fn adaptive_bezier_curves_preserve_endpoints() {
        use super::{stroke_cubic_bezier_adaptive, stroke_quadratic_bezier_adaptive};
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        stroke_quadratic_bezier_adaptive(
            &mut target, (1, 6), (4, 0), (6, 6), 1, 10, Color::RED,
        );
        stroke_cubic_bezier_adaptive(
            &mut target, (0, 0), (0, 7), (7, 7), (7, 0), 1, 10, Color::GREEN,
        );
        drop(target);
        let red = |x: usize, y: usize| memory[(y * 8 + x) * 4] == 255;
        let green = |x: usize, y: usize| memory[(y * 8 + x) * 4 + 1] == 255;
        assert!(red(1, 6));
        assert!(red(6, 6));
        assert!(green(0, 0));
        assert!(green(7, 0));
    }

    #[test]
    fn adaptive_bezier_detects_collinear_control_overshoot() {
        use super::stroke_quadratic_bezier_adaptive;
        let mut memory = [0u8; 8 * 4 * 4];
        let mut target = surface(&mut memory);
        stroke_quadratic_bezier_adaptive(
            &mut target, (1, 1), (10, 1), (5, 1), 0, 10, Color::RED,
        );
        drop(target);
        // The control point lies beyond the endpoint; the curve reverses and
        // reaches pixels that a direct endpoint chord would omit.
        assert!(memory[(1 * 8 + 6) * 4] == 255);
    }

    #[test]
    fn strokes_ellipse_without_filling_its_interior() {
        use super::stroke_ellipse;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        stroke_ellipse(&mut target, 3, 3, 2, 1, Color::GREEN).unwrap();
        drop(target);
        let pixel = |x: usize, y: usize| memory[(y * 8 + x) * 4 + 1] == 255;
        assert!(pixel(1, 3));
        assert!(pixel(5, 3));
        assert!(pixel(3, 2));
        assert!(pixel(3, 4));
        assert!(!pixel(3, 3));
    }
    #[test]
    fn fills_clipped_ellipse_with_symmetric_scanlines() {
        use super::fill_ellipse;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        fill_ellipse(&mut target, 3, 3, 2, 1, Color::RED).unwrap();
        drop(target);
        let pixel = |x: usize, y: usize| memory[(y * 8 + x) * 4] == 255;
        assert!(pixel(3, 3));
        assert!(pixel(1, 3));
        assert!(pixel(5, 3));
        assert!(pixel(3, 2));
        assert!(pixel(3, 4));
        assert!(!pixel(0, 3));
    }

    #[test]
    fn wide_ellipses_scan_on_the_major_axis() {
        use super::{fill_ellipse, stroke_ellipse};

        let mut fill_memory = [0u8; 220 * 5 * 4];
        let info = FramebufferInfo {
            address: 1,
            width: 220,
            height: 5,
            pitch: 220 * 4,
            bits_per_pixel: 32,
            pixel_format: PixelFormat::Rgb,
        };
        // SAFETY: the backing array exactly covers the declared framebuffer.
        let mut fill_target = unsafe {
            FramebufferSurface::from_mapped(
                fill_memory.as_mut_ptr(), fill_memory.len(), info,
            ).unwrap()
        };
        fill_ellipse(&mut fill_target, 110, 2, 100, 2, Color::RED).unwrap();
        drop(fill_target);
        // At y=1 the true horizontal radius is about 86.6 pixels. A
        // row-oriented sqrt truncated before scaling used to lose 36 pixels.
        assert_eq!(fill_memory[(1 * 220 + 30) * 4], 255);
        assert_eq!(fill_memory[(1 * 220 + 190) * 4], 255);
        assert_eq!(fill_memory[(1 * 220 + 10) * 4], 0);

        let mut stroke_memory = [0u8; 220 * 5 * 4];
        // SAFETY: the backing array exactly covers the declared framebuffer.
        let mut stroke_target = unsafe {
            FramebufferSurface::from_mapped(
                stroke_memory.as_mut_ptr(), stroke_memory.len(), info,
            ).unwrap()
        };
        stroke_ellipse(&mut stroke_target, 110, 2, 100, 2, Color::GREEN).unwrap();
        drop(stroke_target);
        assert_eq!(stroke_memory[(1 * 220 + 30) * 4 + 1], 255);
        assert_eq!(stroke_memory[(1 * 220 + 190) * 4 + 1], 255);
        assert_eq!(stroke_memory[(1 * 220 + 10) * 4 + 1], 0);
    }

    #[test]
    fn rejects_unrepresentable_ellipse_radius_before_writing() {
        use super::fill_ellipse;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        assert_eq!(fill_ellipse(&mut target, 3, 3, usize::MAX, 1, Color::RED),
            Err(crate::drivers::framebuffer::info::FramebufferError::SizeOverflow));
        drop(target);
        assert!(memory.iter().all(|byte| *byte == 0));
    }
    #[test]
    fn antialiased_line_blends_fractional_coverage() {
        use super::stroke_line_aa;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        target.clear(Color::BLACK);
        stroke_line_aa(&mut target, (0, 0), (7, 3), Color::WHITE).unwrap();
        drop(target);
        let has_partial = (0..memory.len()).step_by(4).any(|offset| {
            let value = memory[offset];
            value != 0 && value != 255
        });
        assert!(has_partial);
        assert!(memory[0] > 0);
        assert_eq!(memory[3], 0);
    }

    #[test]
    fn antialiased_line_clips_extreme_coordinates_and_vertical_segments() {
        use super::stroke_line_aa;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        target.clear(Color::BLACK);
        stroke_line_aa(
            &mut target,
            (i64::MIN, i64::MIN),
            (i64::MAX, i64::MAX),
            Color::WHITE,
        ).unwrap();
        stroke_line_aa(&mut target, (4, -10), (4, 20), Color::RED).unwrap();
        drop(target);

        for coordinate in 0..8 {
            let diagonal = (coordinate * 8 + coordinate) * 4;
            assert_eq!(memory[diagonal], 255);
            let vertical = (coordinate * 8 + 4) * 4;
            assert_eq!(memory[vertical], 255);
            assert_eq!(memory[vertical + 3], 0);
        }
    }

    #[test]
    fn antialiased_ellipse_renders_all_quadrants_and_rejects_overflow() {
        use super::stroke_ellipse_aa;
        let mut memory = [0u8; 16 * 16 * 4];
        let mut target = surface(&mut memory);
        target.clear(Color::BLACK);
        stroke_ellipse_aa(&mut target, 8, 8, 5, 3, Color::WHITE).unwrap();
        assert_eq!(
            stroke_ellipse_aa(&mut target, i64::MAX, 0, 1, 1, Color::RED),
            Err(FramebufferError::SizeOverflow),
        );
        drop(target);

        for &(x, y) in &[(13usize, 8usize), (8, 11), (3, 8), (8, 5)] {
            let offset = (y * 16 + x) * 4;
            assert!(memory[offset] > 0 || memory[offset + 1] > 0 || memory[offset + 2] > 0);
        }
        assert!(memory.chunks_exact(4).any(|pixel| {
            pixel[0] > 0 && pixel[0] < 255
        }));
    }

    #[test]
    fn antialiased_polyline_and_bezier_curves_render_with_partial_coverage() {
        use super::{stroke_cubic_bezier_aa, stroke_polyline_aa, stroke_quadratic_bezier_aa};
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        target.clear(Color::BLACK);
        stroke_polyline_aa(&mut target, &[(0, 1), (5, 3), (7, 6)], Color::WHITE).unwrap();
        stroke_quadratic_bezier_aa(
            &mut target, (0, 7), (3, 2), (7, 7), 1, 8, Color::RED,
        ).unwrap();
        stroke_cubic_bezier_aa(
            &mut target, (0, 0), (1, 6), (6, 6), (7, 0), 1, 8, Color::GREEN,
        ).unwrap();
        drop(target);
        assert!(memory.chunks_exact(4).any(|pixel| pixel[0] > 0 && pixel[0] < 255));
        assert!(memory.chunks_exact(4).any(|pixel| pixel[1] > 0 && pixel[1] < 255));
        assert!(memory.chunks_exact(4).any(|pixel| pixel[2] > 0 && pixel[2] < 255));
        assert!(memory.chunks_exact(4).all(|pixel| pixel[3] == 0));
    }

    #[test]
    fn strokes_open_polyline_without_closing_last_segment() {
        use super::stroke_polyline;
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        stroke_polyline(&mut target, &[(1, 1), (5, 1), (5, 5)], Color::GREEN);
        drop(target);
        let pixel = |x: usize, y: usize| memory[(y * 8 + x) * 4 + 1] == 255;
        assert!(pixel(3, 1));
        assert!(pixel(5, 3));
        assert!(!pixel(3, 3));
        assert!(!pixel(1, 3));
    }

    #[test]
    fn strokes_closed_polygon_edges() {
        let mut memory = [0u8; 8 * 8 * 4];
        let mut target = surface(&mut memory);
        stroke_polygon(&mut target, &[(1, 1), (5, 1), (5, 5), (1, 5)], Color::GREEN);
        drop(target);
        assert_eq!(memory[(1 * 8 + 3) * 4 + 1], 255);
        assert_eq!(memory[(3 * 8 + 5) * 4 + 1], 255);
        assert_eq!(memory[(3 * 8 + 3) * 4 + 1], 0);
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
    fn clips_extreme_signed_coordinates_without_wrapping() {
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
            Ok(()),
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
