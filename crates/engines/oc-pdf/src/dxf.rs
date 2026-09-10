//! Reading a DXF drawing into paths.
//!
//! # What the spike found
//!
//! Forty-five real DXF files — laser-cutting panels, which is what a `.dxf`
//! usually is — were read before this module was written. Every one is ASCII
//! R14 (`AC1014`), and between them they hold exactly **five** entity types:
//!
//! | | count | what it needs |
//! |---|---|---|
//! | `LWPOLYLINE` | 1036 | vertices, and **845 bulges** — arcs between them |
//! | `CIRCLE` | 434 | centre and radius |
//! | `LINE` | 123 | two points |
//! | `ARC` | 59 | centre, radius, two angles |
//! | `SPLINE` | 25 | control points, knots, weights |
//!
//! That is the whole scope, and it is a closed set rather than a guess. An
//! entity outside it is **counted and named** in the receipt rather than
//! silently skipped, because a drawing that lost its dimensions or its text
//! should not come back looking complete.
//!
//! # A DXF is a stream of pairs
//!
//! Not a nested format: it is `group code` then `value`, two lines each, all
//! the way down. Sections are delimited by pair `(0, SECTION)` and `(2, NAME)`.
//! So this is a scanner over pairs — no recursion, no lookahead beyond the
//! current entity, and an unrecognised code is skipped rather than interpreted.
//!
//! # What is dropped
//!
//! Layers, colours, line types and widths, blocks and their insertions, text,
//! dimensions and hatches. What comes out is the **geometry**: the curves a
//! cutter would follow, which is what a DXF is for.

/// The most entities read from one drawing.
///
/// A bound on a loop over attacker-supplied structure, not an opinion about
/// drawings. The largest file in the spike holds a few hundred.
const MAX_ENTITIES: usize = 200_000;

/// How many straight segments an arc or a spline is flattened into, per full
/// turn or per span.
///
/// PDF draws curves with cubic Béziers and could carry the arcs exactly. This
/// flattens instead, and the reason is that a DXF's curves are of four
/// different kinds — circles, arcs, bulges and rational B-splines — and one
/// flattener that is right about all four beats four exact conversions that
/// each have their own way of being subtly wrong. At 64 segments a full circle
/// the error is under a thousandth of the radius, which is finer than any
/// cutter this file describes.
const SEGMENTS_PER_TURN: usize = 64;

/// A point in the drawing's own units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// One path: a run of points, and whether it closes.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub points: Vec<Point>,
    pub closed: bool,
}

/// A whole drawing.
#[derive(Debug, Default)]
pub struct Drawing {
    pub paths: Vec<Path>,
    /// Entity types found and not drawn, with how many of each.
    pub skipped: Vec<(String, usize)>,
}

impl Drawing {
    /// The bounding box, as `(min_x, min_y, max_x, max_y)`.
    ///
    /// `None` for a drawing with no points, which is a drawing with nothing to
    /// place on a page.
    #[must_use]
    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut b: Option<(f64, f64, f64, f64)> = None;
        for p in self.paths.iter().flat_map(|p| &p.points) {
            b = Some(match b {
                None => (p.x, p.y, p.x, p.y),
                Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
            });
        }
        b
    }
}

/// One `(group code, value)` pair.
type Pair<'a> = (i32, &'a str);

/// Read the pair stream.
///
/// A DXF's codes and values alternate, one per line. A line that is not an
/// integer where a code belongs ends the read: a file that has gone out of step
/// cannot be brought back into it by guessing.
fn pairs(text: &str) -> Vec<Pair<'_>> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let (Some(code), Some(value)) = (lines.next(), lines.next()) {
        let Ok(code) = code.trim().parse::<i32>() else {
            break;
        };
        out.push((code, value.trim()));
    }
    out
}

/// Parse a DXF into paths.
///
/// # Errors
///
/// A file that is not text, or one with no `ENTITIES` section — which is a
/// drawing with nothing in it, and saying so beats returning an empty page.
pub fn read(bytes: &[u8]) -> Result<Drawing, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        "this DXF is not valid UTF-8 text. Binary DXF and the older code pages are not read \
         by this build; export it as ASCII DXF."
            .to_string()
    })?;
    let pairs = pairs(text);

    // Find the ENTITIES section: `(0, SECTION)` then `(2, ENTITIES)`.
    let mut at = None;
    for (i, w) in pairs.windows(2).enumerate() {
        if w[0] == (0, "SECTION") && w[1] == (2, "ENTITIES") {
            at = Some(i + 2);
            break;
        }
    }
    let start = at.ok_or_else(|| {
        "this DXF has no ENTITIES section; there is nothing in it to draw".to_string()
    })?;

    let mut drawing = Drawing::default();
    let mut skipped: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

    // Entities are delimited by code 0. Each is read whole, then converted.
    let mut i = start;
    let mut count = 0usize;
    while i < pairs.len() && count < MAX_ENTITIES {
        let (code, name) = pairs[i];
        if code != 0 {
            i += 1;
            continue;
        }
        if name == "ENDSEC" {
            break;
        }
        // The entity's own pairs run to the next code 0.
        let mut end = i + 1;
        while end < pairs.len() && pairs[end].0 != 0 {
            end += 1;
        }
        let body = &pairs[i + 1..end];
        count += 1;

        match name {
            "LINE" => drawing.paths.extend(line(body)),
            "CIRCLE" => drawing.paths.extend(circle(body)),
            "ARC" => drawing.paths.extend(arc(body)),
            "LWPOLYLINE" => drawing.paths.extend(lwpolyline(body)),
            "SPLINE" => drawing.paths.extend(spline(body)),
            other => {
                *skipped.entry(other.to_string()).or_default() += 1;
            }
        }
        i = end;
    }

    drawing.skipped = skipped.into_iter().collect();
    if drawing.paths.is_empty() {
        return Err(
            "this DXF holds no geometry this build draws. It reads LINE, CIRCLE, ARC, \
             LWPOLYLINE and SPLINE; blocks, text, dimensions and hatches are not drawn."
                .to_string(),
        );
    }
    Ok(drawing)
}

/// The first value for a group code, as a number.
fn num(body: &[Pair<'_>], code: i32) -> Option<f64> {
    body.iter()
        .find(|(c, _)| *c == code)
        .and_then(|(_, v)| v.parse().ok())
}

/// Every value for a group code, as numbers.
fn nums(body: &[Pair<'_>], code: i32) -> Vec<f64> {
    body.iter()
        .filter(|(c, _)| *c == code)
        .filter_map(|(_, v)| v.parse().ok())
        .collect()
}

fn line(body: &[Pair<'_>]) -> Option<Path> {
    Some(Path {
        points: vec![
            Point {
                x: num(body, 10)?,
                y: num(body, 20)?,
            },
            Point {
                x: num(body, 11)?,
                y: num(body, 21)?,
            },
        ],
        closed: false,
    })
}

fn circle(body: &[Pair<'_>]) -> Option<Path> {
    let (cx, cy, r) = (num(body, 10)?, num(body, 20)?, num(body, 40)?);
    Some(Path {
        points: sweep(cx, cy, r, 0.0, std::f64::consts::TAU),
        closed: true,
    })
}

fn arc(body: &[Pair<'_>]) -> Option<Path> {
    let (cx, cy, r) = (num(body, 10)?, num(body, 20)?, num(body, 40)?);
    // DXF states arc angles in DEGREES, counter-clockwise, and the end angle
    // may be numerically smaller than the start — an arc from 350° to 10° is
    // twenty degrees long, not minus three hundred and forty.
    let start = num(body, 50).unwrap_or(0.0).to_radians();
    let mut end = num(body, 51).unwrap_or(360.0).to_radians();
    while end < start {
        end += std::f64::consts::TAU;
    }
    Some(Path {
        points: sweep(cx, cy, r, start, end - start),
        closed: false,
    })
}

/// Points along an arc, flattened.
fn sweep(cx: f64, cy: f64, r: f64, from: f64, span: f64) -> Vec<Point> {
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let steps = ((span.abs() / std::f64::consts::TAU * SEGMENTS_PER_TURN as f64).ceil() as usize)
        .clamp(2, SEGMENTS_PER_TURN * 4);
    (0..=steps)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = from + span * (i as f64 / steps as f64);
            Point {
                x: cx + r * t.cos(),
                y: cy + r * t.sin(),
            }
        })
        .collect()
}

/// A lightweight polyline, whose vertices may carry **bulges**.
///
/// # The bulge is the whole difficulty
///
/// 845 of the 1036 polylines in the spike carry one. A bulge on a vertex says
/// the segment to the NEXT vertex is a circular arc rather than a line, and it
/// encodes that arc as `tan(θ/4)` where θ is the included angle. Drawing those
/// segments straight would turn every rounded corner in every panel into a
/// chamfer — a conversion that looks like it worked.
fn lwpolyline(body: &[Pair<'_>]) -> Option<Path> {
    // The vertices and their bulges are INTERLEAVED in the pair stream, so
    // collecting each code separately would lose which bulge belongs to which
    // vertex — a vertex with no bulge simply has no code 42.
    let mut verts: Vec<(Point, f64)> = Vec::new();
    let mut x: Option<f64> = None;
    for (code, value) in body {
        let Ok(v) = value.parse::<f64>() else {
            continue;
        };
        match code {
            10 => x = Some(v),
            20 => {
                if let Some(x) = x.take() {
                    verts.push((Point { x, y: v }, 0.0));
                }
            }
            42 => {
                if let Some(last) = verts.last_mut() {
                    last.1 = v;
                }
            }
            _ => {}
        }
    }
    if verts.len() < 2 {
        return None;
    }
    let closed = num(body, 70).unwrap_or(0.0) as i64 & 1 == 1;

    let mut points = Vec::with_capacity(verts.len() * 2);
    let n = verts.len();
    for i in 0..n {
        let (a, bulge) = verts[i];
        let next = if i + 1 < n {
            Some(verts[i + 1].0)
        } else if closed {
            Some(verts[0].0)
        } else {
            None
        };
        points.push(a);
        let Some(b) = next else { continue };
        if bulge.abs() > 1e-12 {
            points.extend(bulge_arc(a, b, bulge));
        }
    }
    Some(Path { points, closed })
}

/// The interior points of a bulged segment.
///
/// `bulge = tan(theta / 4)`, signed: positive is counter-clockwise. From it the
/// included angle, the radius and the centre all follow, and the arc is then
/// the same sweep every other curve here uses.
fn bulge_arc(a: Point, b: Point, bulge: f64) -> Vec<Point> {
    let theta = 4.0 * bulge.atan();
    let chord = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt();
    if chord < 1e-12 || theta.abs() < 1e-12 {
        return Vec::new();
    }
    let radius = chord / (2.0 * (theta / 2.0).sin());
    // The centre sits off the chord's midpoint, on the side the sign chooses.
    let (mx, my) = ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let h = (radius.powi(2) - (chord / 2.0).powi(2)).max(0.0).sqrt();
    let (dx, dy) = ((b.x - a.x) / chord, (b.y - a.y) / chord);
    let sign = if bulge > 0.0 { 1.0 } else { -1.0 };
    let (cx, cy) = (mx - dy * h * sign, my + dx * h * sign);

    let start = (a.y - cy).atan2(a.x - cx);
    let swept = sweep(cx, cy, radius.abs(), start, theta);
    // The endpoints are the vertices themselves, which the caller already has.
    swept
        .get(1..swept.len().saturating_sub(1))
        .unwrap_or(&[])
        .to_vec()
}

/// A spline, flattened.
///
/// # Why de Boor rather than the control polygon
///
/// A B-spline does not pass through its control points, so drawing the polygon
/// gives a shape that is recognisably the right idea and measurably wrong —
/// which on a cutting file is the worst kind of wrong. This evaluates the curve
/// properly, weights included, so a rational spline (an exact conic) comes out
/// as the conic and not an approximation of an approximation.
fn spline(body: &[Pair<'_>]) -> Option<Path> {
    let degree = num(body, 71).unwrap_or(3.0).clamp(1.0, 7.0) as usize;
    let knots = nums(body, 40);
    let weights = nums(body, 41);

    let xs = nums(body, 10);
    let ys = nums(body, 20);
    let control: Vec<Point> = xs.iter().zip(&ys).map(|(&x, &y)| Point { x, y }).collect();
    if control.len() <= degree || knots.len() < control.len() + degree + 1 {
        // Not a spline this can evaluate. The control polygon is a lie, so the
        // entity is dropped and the caller counts it.
        return None;
    }

    let closed = num(body, 70).unwrap_or(0.0) as i64 & 1 == 1;
    let lo = knots[degree];
    let hi = knots[control.len()];
    // `partial_cmp` rather than a negated `<`, so a NaN knot is handled by
    // name: `None` from the comparison means the two are incomparable, which
    // for a knot vector means the spline cannot be evaluated at all.
    if lo.partial_cmp(&hi) != Some(std::cmp::Ordering::Less) {
        return None;
    }

    let steps = (control.len() * 8).clamp(16, 1024);
    let points = (0..=steps)
        .map(|i| {
            #[allow(clippy::cast_precision_loss)]
            let t = lo + (hi - lo) * (i as f64 / steps as f64);
            de_boor(t.min(hi - 1e-12), degree, &control, &knots, &weights)
        })
        .collect();
    Some(Path { points, closed })
}

/// One point on a rational B-spline.
fn de_boor(t: f64, degree: usize, control: &[Point], knots: &[f64], weights: &[f64]) -> Point {
    // The span containing `t`.
    let mut k = degree;
    while k + 1 < control.len() && knots[k + 1] <= t {
        k += 1;
    }

    let w = |i: usize| weights.get(i).copied().unwrap_or(1.0);
    // Homogeneous coordinates, so the weights fall out of one division at the
    // end rather than being applied per level.
    let mut d: Vec<(f64, f64, f64)> = (0..=degree)
        .map(|j| {
            let i = k + j - degree;
            let wi = w(i);
            (control[i].x * wi, control[i].y * wi, wi)
        })
        .collect();

    for r in 1..=degree {
        for j in (r..=degree).rev() {
            let i = k + j - degree;
            let lo = knots[i];
            let hi = knots[i + degree + 1 - r];
            let alpha = if (hi - lo).abs() < 1e-12 {
                0.0
            } else {
                (t - lo) / (hi - lo)
            };
            let (px, py, pw) = d[j - 1];
            let (qx, qy, qw) = d[j];
            d[j] = (
                px.mul_add(1.0 - alpha, qx * alpha),
                py.mul_add(1.0 - alpha, qy * alpha),
                pw.mul_add(1.0 - alpha, qw * alpha),
            );
        }
    }
    let (x, y, w) = d[degree];
    if w.abs() < 1e-12 {
        Point { x, y }
    } else {
        Point { x: x / w, y: y / w }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dxf(entities: &str) -> Vec<u8> {
        format!("0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n").into_bytes()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    #[test]
    fn a_line_is_two_points() {
        let d = read(&dxf(
            "0\nLINE\n10\n1.0\n20\n2.0\n30\n0.0\n11\n4.0\n21\n6.0\n31\n0.0\n",
        ))
        .expect("read");
        assert_eq!(d.paths.len(), 1);
        assert_eq!(d.paths[0].points[0], Point { x: 1.0, y: 2.0 });
        assert_eq!(d.paths[0].points[1], Point { x: 4.0, y: 6.0 });
        assert_eq!(d.bounds(), Some((1.0, 2.0, 4.0, 6.0)));
    }

    #[test]
    fn a_circle_is_closed_and_the_right_size() {
        let d = read(&dxf("0\nCIRCLE\n10\n0.0\n20\n0.0\n40\n10.0\n")).expect("read");
        assert!(d.paths[0].closed);
        let (x0, y0, x1, y1) = d.bounds().expect("bounds");
        assert!(close(x0, -10.0) && close(y0, -10.0), "{x0} {y0}");
        assert!(close(x1, 10.0) && close(y1, 10.0), "{x1} {y1}");
        // Every point is on the circle, which the flattening must not change.
        for p in &d.paths[0].points {
            assert!(close(p.x.hypot(p.y), 10.0), "{p:?} is off the circle");
        }
    }

    /// An arc from 350° to 10° is twenty degrees long. Read as `end - start`
    /// it is minus three hundred and forty, and sweeps the wrong way round the
    /// whole circle.
    #[test]
    fn an_arc_wrapping_past_zero_is_the_short_way_round() {
        let d = read(&dxf(
            "0\nARC\n10\n0.0\n20\n0.0\n40\n1.0\n50\n350.0\n51\n10.0\n",
        ))
        .expect("read");
        let pts = &d.paths[0].points;
        assert!(close(pts[0].x, 350f64.to_radians().cos()), "starts at 350");
        let last = pts.last().expect("points");
        assert!(close(last.x, 10f64.to_radians().cos()), "ends at 10");
        // Twenty degrees of a unit circle never goes below y = -0.2.
        assert!(
            pts.iter().all(|p| p.y > -0.25),
            "the arc took the long way round"
        );
    }

    /// **The bulge.** 845 of the spike's 1036 polylines carry one, and drawing
    /// the segment straight turns every rounded corner into a chamfer.
    /// **The bulge, and which way it bows.**
    ///
    /// The specification: the bulge is `tan(theta / 4)`, "made negative if the
    /// arc goes clockwise from the start point to the endpoint". So a POSITIVE
    /// bulge is counterclockwise — and counterclockwise from (0,0) to (1,0)
    /// leaves (0,0) heading downward and comes back up at (1,0), which is the
    /// arc BELOW the chord.
    ///
    /// Worth a test of its own because it is easy to reason the opposite way
    /// round, and getting it wrong mirrors every rounded corner in the drawing
    /// while leaving it looking entirely plausible.
    #[test]
    fn a_positive_bulge_is_the_counterclockwise_arc() {
        // Two vertices a unit apart with bulge 1: a half-circle of radius 0.5.
        let d = read(&dxf(
            "0\nLWPOLYLINE\n90\n2\n70\n0\n10\n0.0\n20\n0.0\n42\n1.0\n10\n1.0\n20\n0.0\n",
        ))
        .expect("read");
        let pts = &d.paths[0].points;
        assert!(pts.len() > 4, "the arc must add interior points: {pts:?}");

        // Counterclockwise, so below the chord.
        let lowest = pts.iter().map(|p| p.y).fold(f64::MAX, f64::min);
        assert!(close(lowest, -0.5), "lowest at {lowest}, expected -0.5");
        assert!(
            pts.iter().all(|p| p.y <= 1e-9),
            "a positive bulge must not bow upward"
        );
        // Every interior point is on the circle the bulge describes.
        for p in &pts[1..pts.len() - 1] {
            assert!(
                close((p.x - 0.5).hypot(p.y), 0.5),
                "{p:?} is off the bulge arc"
            );
        }
    }

    /// A negative bulge is the same arc mirrored, and the ENDPOINT is what
    /// proves the centre was placed right — a half-circle puts the centre on
    /// the chord's midpoint whichever way it bows, so it cannot test that.
    #[test]
    fn a_bulged_arc_ends_exactly_at_the_next_vertex() {
        // tan(90 degrees / 4) = 0.414214: a quarter turn, whose centre sits
        // half a unit off the chord.
        for (bulge, below) in [("0.41421356", true), ("-0.41421356", false)] {
            let d = read(&dxf(&format!(
                "0\nLWPOLYLINE\n90\n2\n70\n0\n10\n0.0\n20\n0.0\n42\n{bulge}\n10\n1.0\n20\n0.0\n"
            )))
            .expect("read");
            let pts = &d.paths[0].points;

            let last = pts.last().expect("points");
            assert!(
                close(last.x, 1.0) && close(last.y, 0.0),
                "the arc must end on the next vertex, ended at {last:?}"
            );
            let mid = pts[pts.len() / 2];
            assert_eq!(
                mid.y < 0.0,
                below,
                "bulge {bulge} bowed the wrong way: {mid:?}"
            );
            // A quarter turn on a unit chord has radius 1/sqrt(2), centred
            // half a unit above or below the chord's midpoint.
            let cy = if below { 0.5 } else { -0.5 };
            for p in pts {
                assert!(
                    close((p.x - 0.5).hypot(p.y - cy), std::f64::consts::FRAC_1_SQRT_2),
                    "{p:?} is not on the quarter arc"
                );
            }
        }
    }

    #[test]
    fn a_polyline_with_no_bulge_keeps_its_vertices_and_its_closed_flag() {
        let d = read(&dxf(
            "0\nLWPOLYLINE\n90\n3\n70\n1\n10\n0.0\n20\n0.0\n10\n2.0\n20\n0.0\n10\n2.0\n20\n2.0\n",
        ))
        .expect("read");
        assert!(d.paths[0].closed);
        assert_eq!(d.paths[0].points.len(), 3);
        assert_eq!(d.bounds(), Some((0.0, 0.0, 2.0, 2.0)));
    }

    /// A B-spline does not pass through its control points, so the curve must
    /// be evaluated rather than the polygon drawn.
    #[test]
    fn a_spline_is_evaluated_not_approximated_by_its_control_polygon() {
        // A quadratic spline whose middle control point is well off the curve.
        let d = read(&dxf("0\nSPLINE\n70\n8\n71\n2\n72\n6\n73\n3\n\
             40\n0.0\n40\n0.0\n40\n0.0\n40\n1.0\n40\n1.0\n40\n1.0\n\
             10\n0.0\n20\n0.0\n10\n1.0\n20\n2.0\n10\n2.0\n20\n0.0\n"))
        .expect("read");
        let pts = &d.paths[0].points;
        assert!(close(pts[0].x, 0.0) && close(pts[0].y, 0.0), "{:?}", pts[0]);
        let last = pts.last().expect("points");
        assert!(close(last.x, 2.0) && close(last.y, 0.0), "{last:?}");
        // The apex of this curve is at y = 1, not at the control point's y = 2.
        let peak = pts.iter().map(|p| p.y).fold(f64::MIN, f64::max);
        assert!(close(peak, 1.0), "apex at {peak}, expected 1.0 not 2.0");
    }

    /// What is not drawn is counted and named, so a drawing that lost its
    /// dimensions does not come back looking complete.
    #[test]
    fn unread_entities_are_counted_by_name() {
        let d = read(&dxf("0\nLINE\n10\n0\n20\n0\n11\n1\n21\n1\n\
             0\nTEXT\n1\nhello\n\
             0\nDIMENSION\n\
             0\nTEXT\n1\nagain\n"))
        .expect("read");
        assert_eq!(d.paths.len(), 1);
        assert_eq!(
            d.skipped,
            vec![("DIMENSION".to_string(), 1), ("TEXT".to_string(), 2)]
        );
    }

    #[test]
    fn a_drawing_with_nothing_drawable_is_refused_rather_than_returned_empty() {
        let err = read(&dxf("0\nTEXT\n1\nonly text\n")).expect_err("nothing to draw");
        assert!(err.contains("no geometry"), "{err}");
        let err = read(b"not a dxf at all").expect_err("no ENTITIES");
        assert!(err.contains("ENTITIES"), "{err}");
    }
}
