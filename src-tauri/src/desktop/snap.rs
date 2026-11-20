use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::config::schema::{PointF, RectF};

pub const DEFAULT_MARGIN: f64 = 20.0;
pub const DEFAULT_GAP: f64 = 12.0;
pub const DEFAULT_THRESHOLD: f64 = 10.0;
pub const DEFAULT_GRID: f64 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub enum GuideAxis {
    X,
    Y,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct Guide {
    pub axis: GuideAxis,
    pub pos: f64,
    pub from: f64,
    pub to: f64,
    pub dashed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/bindings/")]
pub struct EditGuides {
    pub monitor: String,
    pub lines: Vec<Guide>,
}

#[derive(Debug, Clone)]
pub struct SnapCtx {
    pub work: RectF,
    pub others: Vec<RectF>,
    pub margin: f64,
    pub gap: f64,
    pub threshold: f64,
    pub grid: Option<f64>,
    pub snap_enabled: bool,
}

impl SnapCtx {
    pub fn standard(work: RectF, others: Vec<RectF>, grid_snap: bool, snap_enabled: bool) -> Self {
        Self {
            work,
            others,
            margin: DEFAULT_MARGIN,
            gap: DEFAULT_GAP,
            threshold: DEFAULT_THRESHOLD,
            grid: if grid_snap { Some(DEFAULT_GRID) } else { None },
            snap_enabled,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum MatchSpan {
    Work,
    Other(usize),
}

#[derive(Debug, Clone, Copy)]
struct AxisMatch {
    delta_abs: f64,
    origin_coord: f64,
    guide_pos: f64,
    dashed: bool,
    span: MatchSpan,
}

fn consider_match(best: &mut Option<AxisMatch>, cand: AxisMatch, threshold: f64) {
    if cand.delta_abs > threshold {
        return;
    }
    match best {
        None => *best = Some(cand),
        Some(prev) if cand.delta_abs < prev.delta_abs - 1e-6 => *best = Some(cand),
        _ => {}
    }
}

fn resolve_axis(
    pos: f64,
    len: f64,
    work_start: f64,
    work_len: f64,
    other_segments: impl Iterator<Item = (usize, f64, f64)>,
    ctx: &SnapCtx,
) -> (f64, Option<AxisMatch>) {
    let max_pos = (work_start + work_len - len).max(work_start);
    if !ctx.snap_enabled {
        return (pos.clamp(work_start, max_pos), None);
    }

    let mut best: Option<AxisMatch> = None;
    let threshold = ctx.threshold.max(0.0);

    if threshold > 0.0 {
        let margin_start = work_start + ctx.margin;
        let margin_end = work_start + work_len - ctx.margin;
        let work_center = work_start + work_len / 2.0;

        // Kanter + centrum for work area
        consider_match(
            &mut best,
            AxisMatch {
                delta_abs: (pos - margin_start).abs(),
                origin_coord: margin_start,
                guide_pos: margin_start,
                dashed: false,
                span: MatchSpan::Work,
            },
            threshold,
        );

        let cur_end = pos + len;
        consider_match(
            &mut best,
            AxisMatch {
                delta_abs: (cur_end - margin_end).abs(),
                origin_coord: margin_end - len,
                guide_pos: margin_end,
                dashed: false,
                span: MatchSpan::Work,
            },
            threshold,
        );

        let cur_center = pos + len / 2.0;
        consider_match(
            &mut best,
            AxisMatch {
                delta_abs: (cur_center - work_center).abs(),
                origin_coord: work_center - len / 2.0,
                guide_pos: work_center,
                dashed: true,
                span: MatchSpan::Work,
            },
            threshold,
        );

        // Andre widgets (kanter, center, samt 12 px gap)
        for (idx, other_start, other_len) in other_segments {
            let other_end = other_start + other_len;
            let other_center = other_start + other_len / 2.0;

            // Vores start mod andens start
            consider_match(
                &mut best,
                AxisMatch {
                    delta_abs: (pos - other_start).abs(),
                    origin_coord: other_start,
                    guide_pos: other_start,
                    dashed: false,
                    span: MatchSpan::Other(idx),
                },
                threshold,
            );

            // Vores slut mod andens slut
            consider_match(
                &mut best,
                AxisMatch {
                    delta_abs: (cur_end - other_end).abs(),
                    origin_coord: other_end - len,
                    guide_pos: other_end,
                    dashed: false,
                    span: MatchSpan::Other(idx),
                },
                threshold,
            );

            // Vores center mod andens center
            consider_match(
                &mut best,
                AxisMatch {
                    delta_abs: (cur_center - other_center).abs(),
                    origin_coord: other_center - len / 2.0,
                    guide_pos: other_center,
                    dashed: true,
                    span: MatchSpan::Other(idx),
                },
                threshold,
            );

            // Vores start = andens slut + gap
            let after_other = other_end + ctx.gap;
            consider_match(
                &mut best,
                AxisMatch {
                    delta_abs: (pos - after_other).abs(),
                    origin_coord: after_other,
                    guide_pos: after_other,
                    dashed: false,
                    span: MatchSpan::Other(idx),
                },
                threshold,
            );

            // Vores slut = andens start - gap
            let before_other = other_start - ctx.gap;
            consider_match(
                &mut best,
                AxisMatch {
                    delta_abs: (cur_end - before_other).abs(),
                    origin_coord: before_other - len,
                    guide_pos: before_other,
                    dashed: false,
                    span: MatchSpan::Other(idx),
                },
                threshold,
            );
        }
    }

    let raw_coord = match best {
        Some(m) => m.origin_coord,
        None => {
            if let Some(grid) = ctx.grid.filter(|g| *g > 0.0) {
                let origin = work_start + ctx.margin;
                origin + ((pos - origin) / grid).round() * grid
            } else {
                pos
            }
        }
    };

    let clamped = raw_coord.clamp(work_start, max_pos);
    let active_match = best.filter(|_| (clamped - raw_coord).abs() < 0.5);
    (clamped, active_match)
}

pub fn resolve(c: RectF, ctx: &SnapCtx) -> (PointF, Vec<Guide>) {
    let (x, match_x) = resolve_axis(
        c.x,
        c.width,
        ctx.work.x,
        ctx.work.width,
        ctx.others.iter().enumerate().map(|(i, o)| (i, o.x, o.width)),
        ctx,
    );

    let (y, match_y) = resolve_axis(
        c.y,
        c.height,
        ctx.work.y,
        ctx.work.height,
        ctx.others.iter().enumerate().map(|(i, o)| (i, o.y, o.height)),
        ctx,
    );

    let mut guides = Vec::with_capacity(2);

    if let Some(mx) = match_x {
        let (from, to) = match mx.span {
            MatchSpan::Work => (ctx.work.y, ctx.work.y + ctx.work.height),
            MatchSpan::Other(idx) => {
                let other = ctx.others[idx];
                (y.min(other.y), (y + c.height).max(other.y + other.height))
            }
        };
        guides.push(Guide {
            axis: GuideAxis::X,
            pos: mx.guide_pos,
            from,
            to,
            dashed: mx.dashed,
        });
    }

    if let Some(my) = match_y {
        let (from, to) = match my.span {
            MatchSpan::Work => (ctx.work.x, ctx.work.x + ctx.work.width),
            MatchSpan::Other(idx) => {
                let other = ctx.others[idx];
                (x.min(other.x), (x + c.width).max(other.x + other.width))
            }
        };
        guides.push(Guide {
            axis: GuideAxis::Y,
            pos: my.guide_pos,
            from,
            to,
            dashed: my.dashed,
        });
    }

    (PointF { x, y }, guides)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_1080() -> RectF {
        RectF {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1032.0,
        }
    }

    fn widget_at(x: f64, y: f64) -> RectF {
        RectF {
            x,
            y,
            width: 276.0,
            height: 132.0,
        }
    }

    #[test]
    fn snaps_to_work_area_margins() {
        let ctx = SnapCtx::standard(work_1080(), Vec::new(), true, true);

        // Venstre + top kant inden for threshold (10 px) af margin (20 px)
        let (pt, guides) = resolve(widget_at(26.0, 14.0), &ctx);
        assert_eq!(pt, PointF { x: 20.0, y: 20.0 });
        assert_eq!(guides.len(), 2);
        assert_eq!(guides[0].axis, GuideAxis::X);
        assert_eq!(guides[0].pos, 20.0);
        assert!(!guides[0].dashed);
        assert_eq!(guides[1].axis, GuideAxis::Y);
        assert_eq!(guides[1].pos, 20.0);
        assert!(!guides[1].dashed);

        // Højre + bund kant inden for threshold af 1920 - 20 = 1900 / 1032 - 20 = 1012
        let (pt_rb, guides_rb) = resolve(widget_at(1900.0 - 276.0 - 6.0, 1012.0 - 132.0 + 7.0), &ctx);
        assert_eq!(
            pt_rb,
            PointF {
                x: 1624.0,
                y: 880.0
            }
        );
        assert_eq!(guides_rb.len(), 2);
        assert_eq!(guides_rb[0].pos, 1900.0);
        assert_eq!(guides_rb[1].pos, 1012.0);
    }

    #[test]
    fn snaps_to_work_area_center_with_dashed_guides() {
        let ctx = SnapCtx::standard(work_1080(), Vec::new(), true, true);
        let cx = 1920.0 / 2.0 - 276.0 / 2.0; // 822.0
        let cy = 1032.0 / 2.0 - 132.0 / 2.0; // 450.0

        let (pt, guides) = resolve(widget_at(cx + 5.0, cy - 8.0), &ctx);
        assert_eq!(pt, PointF { x: cx, y: cy });
        assert_eq!(guides.len(), 2);
        assert!(guides[0].dashed);
        assert_eq!(guides[0].pos, 960.0);
        assert!(guides[1].dashed);
        assert_eq!(guides[1].pos, 516.0);
    }

    #[test]
    fn snaps_to_other_widget_with_12px_gap_and_edge_align() {
        let other = widget_at(500.0, 200.0);
        let ctx = SnapCtx::standard(work_1080(), vec![other], true, true);

        // Placer til højre for `other`: 500 + 276 + 12 = 788, og Y justeret mod top (200)
        let (pt, guides) = resolve(widget_at(793.0, 205.0), &ctx);
        assert_eq!(pt, PointF { x: 788.0, y: 200.0 });
        assert_eq!(guides.len(), 2);
        assert_eq!(guides[0].axis, GuideAxis::X);
        assert_eq!(guides[0].pos, 788.0);
        assert_eq!(guides[0].from, 200.0);
        assert_eq!(guides[0].to, 332.0);

        // Placer over `other` med 12 px vertikal afstand: other.y - 12 - height = 56
        let (pt_above, guides_above) = resolve(widget_at(504.0, 51.0), &ctx);
        assert_eq!(pt_above, PointF { x: 500.0, y: 56.0 });
        assert_eq!(guides_above.len(), 2);
        assert_eq!(guides_above[1].axis, GuideAxis::Y);
        assert_eq!(guides_above[1].pos, 188.0);
    }

    #[test]
    fn alt_disables_snap_and_grid_while_preserving_clamp() {
        let other = widget_at(500.0, 200.0);
        let ctx = SnapCtx::standard(work_1080(), vec![other], true, false);

        // Ellers ville x=25 snappe til 20 og y=205 snappe til 200
        let (pt, guides) = resolve(widget_at(25.3, 205.7), &ctx);
        assert_eq!(pt, PointF { x: 25.3, y: 205.7 });
        assert!(guides.is_empty());

        // Clamp gælder stadig selv når Alt holder snap slukket
        let (clamped, guides_c) = resolve(widget_at(-40.0, 9999.0), &ctx);
        assert_eq!(
            clamped,
            PointF {
                x: 0.0,
                y: 1032.0 - 132.0
            }
        );
        assert!(guides_c.is_empty());
    }

    #[test]
    fn rounds_to_8px_grid_relative_to_margin_when_no_snap_match() {
        let ctx = SnapCtx::standard(work_1080(), Vec::new(), true, true);
        // Margin = 20 -> 20 + 8 * 20 = 180, 20 + 8 * 30 = 260
        let (pt, guides) = resolve(widget_at(183.0, 257.0), &ctx);
        assert_eq!(pt, PointF { x: 180.0, y: 260.0 });
        assert!(guides.is_empty());
    }

    #[test]
    fn clamps_to_work_area_edges() {
        let ctx = SnapCtx::standard(work_1080(), Vec::new(), true, true);
        let (pt, _) = resolve(widget_at(-120.0, -50.0), &ctx);
        assert_eq!(pt, PointF { x: 0.0, y: 0.0 });

        let (pt2, _) = resolve(widget_at(2500.0, 1500.0), &ctx);
        assert_eq!(
            pt2,
            PointF {
                x: 1920.0 - 276.0,
                y: 1032.0 - 132.0
            }
        );
    }
}
