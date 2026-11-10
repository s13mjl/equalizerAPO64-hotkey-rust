//! GDI and GDI+ painting primitives shared by the native modules.

use super::*;

/// Starts GDI+ once for anti-aliased plot curves.
pub(crate) fn ensure_gdiplus() {
    static TOKEN: OnceLock<usize> = OnceLock::new();
    TOKEN.get_or_init(|| unsafe {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0usize;
        let _ = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        token
    });
}

pub(crate) unsafe fn fill(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32, color: COLORREF) {
    if right <= left || bottom <= top {
        return;
    }
    let rect = RECT { left, top, right, bottom };
    let brush = HBRUSH(GetStockObject(DC_BRUSH).0);
    SetDCBrushColor(hdc, color);
    let _ = FillRect(hdc, &rect, brush);
}

pub(crate) unsafe fn draw_text(hdc: HDC, x: i32, y: i32, text: &[u16], color: COLORREF) {
    if text.is_empty() {
        return;
    }
    SetTextColor(hdc, color);
    let _ = TextOutW(hdc, x, y, text);
}

pub(crate) fn argb_color(color: COLORREF) -> u32 {
    let value = color.0;
    0xFF00_0000
        | ((value & 0x0000_00FF) << 16)
        | (value & 0x0000_FF00)
        | ((value >> 16) & 0x0000_00FF)
}

/// GDI+ draws in device coordinates and ignores the GDI viewport origin used
/// by the double buffer, so the origin is cleared while GDI+ is active and
/// callers pass points in client coordinates (shifted here by `offset`).
pub(crate) struct AaContext {
    graphics: *mut GpGraphics,
    previous: POINT,
    dx: f32,
    dy: f32,
}

pub(crate) unsafe fn begin_aa(hdc: HDC, offset: (i32, i32), clip: RECT) -> Option<AaContext> {
    ensure_gdiplus();
    let mut previous = POINT::default();
    let _ = SetViewportOrgEx(hdc, 0, 0, Some(&mut previous));
    let mut graphics: *mut GpGraphics = std::ptr::null_mut();
    let _ = GdipCreateFromHDC(hdc, &mut graphics);
    if graphics.is_null() {
        let _ = SetViewportOrgEx(hdc, previous.x, previous.y, None);
        return None;
    }
    let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
    let _ = GdipSetPixelOffsetMode(graphics, PixelOffsetModeHighQuality);
    // Intersect with the DC clip so curves in partially scrolled rows stay
    // below the header; an empty intersection still gets a clip so nothing
    // is drawn.
    let mut dc_clip = RECT::default();
    let _ = GetClipBox(hdc, &mut dc_clip);
    let left = (clip.left - offset.0).max(dc_clip.left);
    let top = (clip.top - offset.1).max(dc_clip.top);
    let right = (clip.right - offset.0).min(dc_clip.right);
    let bottom = (clip.bottom - offset.1).min(dc_clip.bottom);
    let _ = GdipSetClipRect(
        graphics,
        left as f32,
        top as f32,
        (right - left).max(0) as f32,
        (bottom - top).max(0) as f32,
        CombineModeReplace,
    );
    Some(AaContext {
        graphics,
        previous,
        dx: offset.0 as f32,
        dy: offset.1 as f32,
    })
}

pub(crate) unsafe fn end_aa(hdc: HDC, context: AaContext) {
    let _ = GdipDeleteGraphics(context.graphics);
    let _ = SetViewportOrgEx(hdc, context.previous.x, context.previous.y, None);
}

pub(crate) unsafe fn draw_aa_runs(hdc: HDC, offset: (i32, i32), runs: &[(COLORREF, Vec<PointF>)], clip: RECT) {
    if runs.is_empty() {
        return;
    }
    if let Some(context) = begin_aa(hdc, offset, clip) {
        for (color, points) in runs {
            if points.len() < 2 {
                continue;
            }
            let shifted: Vec<PointF> = points
                .iter()
                .map(|point| PointF {
                    X: point.X - context.dx,
                    Y: point.Y - context.dy,
                })
                .collect();
            let mut path: *mut GpPath = std::ptr::null_mut();
            let _ = GdipCreatePath(FillModeAlternate, &mut path);
            if path.is_null() {
                continue;
            }
            let _ = GdipAddPathLine2(path, shifted.as_ptr(), shifted.len() as i32);
            let mut pen: *mut GpPen = std::ptr::null_mut();
            let _ = GdipCreatePen1(argb_color(*color), 1.0, UnitPixel, &mut pen);
            if !pen.is_null() {
                let _ = GdipDrawPath(context.graphics, pen, path);
                let _ = GdipDeletePen(pen);
            }
            let _ = GdipDeletePath(path);
        }
        end_aa(hdc, context);
        return;
    }
    // Never lose the curve if GDI+ is unavailable: plain GDI polyline.
    let saved = SaveDC(hdc);
    let _ = IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);
    for (color, points) in runs {
        if points.len() < 2 {
            continue;
        }
        let gdi_points: Vec<POINT> = points
            .iter()
            .map(|point| POINT {
                x: point.X as i32,
                y: point.Y as i32,
            })
            .collect();
        let pen = CreatePen(PS_SOLID, 1, *color);
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = Polyline(hdc, &gdi_points);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
    let _ = RestoreDC(hdc, saved);
}

/// Fills each open run down to `baseline` as one closed path so the top edge
/// stays anti-aliased as well.
pub(crate) unsafe fn draw_aa_filled_areas(
    hdc: HDC,
    offset: (i32, i32),
    areas: &[Vec<PointF>],
    baseline: f32,
    clip: RECT,
    color: COLORREF,
) {
    if areas.is_empty() {
        return;
    }
    if let Some(context) = begin_aa(hdc, offset, clip) {
        for points in areas {
            if points.len() < 2 {
                continue;
            }
            let first = points[0];
            let last = *points.last().unwrap();
            let mut shape: Vec<PointF> = points
                .iter()
                .map(|point| PointF {
                    X: point.X - context.dx,
                    Y: point.Y - context.dy,
                })
                .collect();
            shape.push(PointF {
                X: last.X - context.dx,
                Y: baseline - context.dy,
            });
            shape.push(PointF {
                X: first.X - context.dx,
                Y: baseline - context.dy,
            });
            let mut path: *mut GpPath = std::ptr::null_mut();
            let _ = GdipCreatePath(FillModeAlternate, &mut path);
            if path.is_null() {
                continue;
            }
            let _ = GdipAddPathLine2(path, shape.as_ptr(), shape.len() as i32);
            let _ = GdipClosePathFigure(path);
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            let _ = GdipCreateSolidFill(argb_color(color), &mut brush);
            if !brush.is_null() {
                let _ = GdipFillPath(context.graphics, brush as *mut GpBrush, path);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
            let _ = GdipDeletePath(path);
        }
        end_aa(hdc, context);
        return;
    }
    // Fallback: one rectangle per sampled column, clipped to the plot.
    let saved = SaveDC(hdc);
    let _ = IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);
    let baseline_y = baseline.round() as i32;
    for points in areas {
        for point in points {
            let x = point.X.floor() as i32;
            let top = (point.Y.round() as i32).min(baseline_y);
            fill(hdc, x, top, x + 1, baseline_y, color);
        }
    }
    let _ = RestoreDC(hdc, saved);
}
