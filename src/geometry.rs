//! Pure geometry: window placement and animation paths.
//!
//! All coordinates are absolute (layout) coordinates, i.e. the output's
//! offset is already included, so they can be used with
//! `move absolute position`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl From<swayipc::Rect> for Rect {
    fn from(r: swayipc::Rect) -> Self {
        Self {
            x: r.x,
            y: r.y,
            w: r.width,
            h: r.height,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dim {
    Px(i32),
    Percent(i32),
}

impl Dim {
    fn resolve(self, total: i32) -> i32 {
        match self {
            Dim::Px(px) => px,
            Dim::Percent(p) => (i64::from(total) * i64::from(p) / 100)
                .clamp(i64::from(i32::MIN), i64::from(i32::MAX))
                as i32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub w: Dim,
    pub h: Dim,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Start,
    Centre,
    End,
}

impl Align {
    fn offset(self, space: i32, len: i32) -> i32 {
        match self {
            Align::Start => 0,
            Align::Centre => (space - len) / 2,
            Align::End => space - len,
        }
    }
}

/// Vertical then horizontal alignment, e.g. `tc` = top / centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Anchor {
    pub v: Align,
    pub h: Align,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Computes the window rectangle on `output`.
pub fn resolve(output: Rect, size: Size, pos: (i32, i32), anchor: Anchor) -> Rect {
    let w = size.w.resolve(output.w);
    let h = size.h.resolve(output.h);
    Rect {
        x: output.x + pos.0 + anchor.h.offset(output.w, w),
        y: output.y + pos.1 + anchor.v.offset(output.h, h),
        w,
        h,
    }
}

/// Where the show animation starts: just inside the given screen edge,
/// leaving a single pixel of the window visible.
pub fn show_start(edge: Edge, target: Rect, output: Rect) -> (i32, i32) {
    match edge {
        Edge::Top => (target.x, output.y - target.h + 1),
        Edge::Bottom => (target.x, output.y + output.h - 1),
        Edge::Left => (output.x - target.w + 1, target.y),
        Edge::Right => (output.x + output.w - 1, target.y),
    }
}

/// Where the hide animation ends: fully past the given screen edge.
pub fn hide_end(edge: Edge, target: Rect, output: Rect) -> (i32, i32) {
    match edge {
        Edge::Top => (target.x, output.y - target.h),
        Edge::Bottom => (target.x, output.y + output.h),
        Edge::Left => (output.x - target.w, target.y),
        Edge::Right => (output.x + output.w, target.y),
    }
}

/// Safety net; real paths are a few dozen frames at most.
const MAX_FRAMES: usize = 500;

/// Intermediate positions moving from `from` to `to`, ending exactly on `to`.
///
/// Ports the easing of the original script: showing starts with a quarter of
/// the distance per step and decelerates (÷1.25); hiding starts with a
/// twentieth and accelerates (×1.25).
pub fn frames(from: (i32, i32), to: (i32, i32), show: bool) -> Vec<(i32, i32)> {
    let divisor = if show { 4 } else { 20 };
    let mut pos = from;
    let mut inc = ((to.0 - from.0) / divisor, (to.1 - from.1) / divisor);
    let mut out = Vec::new();

    let remaining = |p: i32, i: i32, end: i32| (i > 0 && p < end) || (i < 0 && p > end);
    while out.len() < MAX_FRAMES && (remaining(pos.0, inc.0, to.0) || remaining(pos.1, inc.1, to.1))
    {
        out.push(pos);
        pos = (pos.0 + inc.0, pos.1 + inc.1);
        inc = if show {
            (inc.0 * 100 / 125, inc.1 * 100 / 125)
        } else {
            (inc.0 * 125 / 100, inc.1 * 125 / 100)
        };
    }
    out.push(to);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: Rect = Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };
    const SECOND: Rect = Rect {
        x: 1920,
        y: 200,
        w: 2560,
        h: 1440,
    };
    const HALF: Size = Size {
        w: Dim::Percent(50),
        h: Dim::Percent(50),
    };

    fn anchor(v: Align, h: Align) -> Anchor {
        Anchor { v, h }
    }

    #[test]
    fn centre_centre_default() {
        let r = resolve(OUT, HALF, (0, 0), anchor(Align::Centre, Align::Centre));
        assert_eq!(
            r,
            Rect {
                x: 480,
                y: 270,
                w: 960,
                h: 540
            }
        );
    }

    #[test]
    fn top_centre_sixty_percent() {
        let size = Size {
            w: Dim::Percent(60),
            h: Dim::Percent(60),
        };
        let r = resolve(OUT, size, (0, 0), anchor(Align::Start, Align::Centre));
        assert_eq!(
            r,
            Rect {
                x: 384,
                y: 0,
                w: 1152,
                h: 648
            }
        );
    }

    #[test]
    fn top_right_pixels_with_offset_output() {
        let size = Size {
            w: Dim::Px(900),
            h: Dim::Px(500),
        };
        let r = resolve(SECOND, size, (-10, 20), anchor(Align::Start, Align::End));
        assert_eq!(
            r,
            Rect {
                x: 1920 + 2560 - 900 - 10,
                y: 220,
                w: 900,
                h: 500
            }
        );
    }

    #[test]
    fn bottom_left() {
        let r = resolve(SECOND, HALF, (5, -5), anchor(Align::End, Align::Start));
        assert_eq!(
            r,
            Rect {
                x: 1925,
                y: 200 + 720 - 5,
                w: 1280,
                h: 720
            }
        );
    }

    fn check_path(path: &[(i32, i32)], from: (i32, i32), to: (i32, i32)) {
        assert_eq!(*path.last().unwrap(), to);
        assert!(path.len() < 100, "too many frames: {}", path.len());
        if path.len() > 1 {
            assert_eq!(path[0], from);
        }
        // Monotonic towards the target on both axes.
        for w in path.windows(2) {
            assert!((w[1].0 - w[0].0).signum() * (to.0 - from.0).signum() >= 0);
            assert!((w[1].1 - w[0].1).signum() * (to.1 - from.1).signum() >= 0);
        }
    }

    #[test]
    fn animations_terminate_on_target() {
        for output in [OUT, SECOND] {
            let target = resolve(output, HALF, (0, 0), anchor(Align::Start, Align::Centre));
            for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                let start = show_start(edge, target, output);
                check_path(
                    &frames(start, (target.x, target.y), true),
                    start,
                    (target.x, target.y),
                );
                let end = hide_end(edge, target, output);
                check_path(
                    &frames((target.x, target.y), end, false),
                    (target.x, target.y),
                    end,
                );
            }
        }
    }

    #[test]
    fn zero_distance_is_single_frame() {
        assert_eq!(frames((5, 5), (5, 5), true), vec![(5, 5)]);
        assert_eq!(frames((5, 5), (7, 5), false), vec![(7, 5)]);
    }

    #[test]
    fn hidden_window_is_fully_offscreen() {
        let target = resolve(SECOND, HALF, (0, 0), anchor(Align::Centre, Align::Centre));
        let (_, y) = hide_end(Edge::Top, target, SECOND);
        assert!(y + target.h <= SECOND.y);
        let (x, _) = hide_end(Edge::Right, target, SECOND);
        assert!(x >= SECOND.x + SECOND.w);
    }
}
