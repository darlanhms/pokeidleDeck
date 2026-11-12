use crate::config::LayoutMode;
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Divide `area` entre `n` contas sem deixar buraco: 1 ocupa tudo, 2 lado a lado (ou empilhadas),
/// 3 com duas em cima e a terceira esticada embaixo, 4 em grade 2x2.
pub fn grid(n: usize, area: Rect, stacked: bool, gap: f64) -> Vec<Rect> {
    let half_w = ((area.w - gap) / 2.0).max(0.0);
    let half_h = ((area.h - gap) / 2.0).max(0.0);
    let right = area.x + half_w + gap;
    let bottom = area.y + half_h + gap;
    let cell = |x, y, w, h| Rect { x, y, w, h };
    match n {
        0 => vec![],
        1 => vec![area],
        2 if stacked => vec![
            cell(area.x, area.y, area.w, half_h),
            cell(area.x, bottom, area.w, half_h),
        ],
        2 => vec![
            cell(area.x, area.y, half_w, area.h),
            cell(right, area.y, half_w, area.h),
        ],
        3 => vec![
            cell(area.x, area.y, half_w, half_h),
            cell(right, area.y, half_w, half_h),
            cell(area.x, bottom, area.w, half_h),
        ],
        _ => vec![
            cell(area.x, area.y, half_w, half_h),
            cell(right, area.y, half_w, half_h),
            cell(area.x, bottom, half_w, half_h),
            cell(right, bottom, half_w, half_h),
        ],
    }
}

/// N colunas iguais, lado a lado.
pub fn columns(n: usize, area: Rect, gap: f64) -> Vec<Rect> {
    if n == 0 {
        return vec![];
    }
    let w = ((area.w - gap * (n as f64 - 1.0)) / n as f64).max(0.0);
    (0..n)
        .map(|i| Rect { x: area.x + i as f64 * (w + gap), y: area.y, w, h: area.h })
        .collect()
}

/// Onde fica cada conta; `None` é conta escondida (as não ampliadas, no modo ampliada).
pub fn arrange(mode: LayoutMode, n: usize, focused: Option<usize>, area: Rect, stacked: bool, gap: f64) -> Vec<Option<Rect>> {
    match (mode, focused) {
        (LayoutMode::Focus, Some(index)) => (0..n).map(|i| (i == index).then_some(area)).collect(),
        (LayoutMode::Columns, _) => columns(n, area, gap).into_iter().map(Some).collect(),
        _ => grid(n, area, stacked, gap).into_iter().map(Some).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AREA: Rect = Rect { x: 0.0, y: 40.0, w: 1000.0, h: 600.0 };

    fn covered(rects: &[Rect], gap: f64) -> f64 {
        rects.iter().map(|r| r.w * r.h).sum::<f64>() + gap
    }

    #[test]
    fn one_fills_everything() {
        assert_eq!(grid(1, AREA, false, 4.0), vec![AREA]);
    }

    #[test]
    fn two_side_by_side_or_stacked() {
        let side = grid(2, AREA, false, 0.0);
        assert_eq!(side[0].h, AREA.h);
        assert_eq!(side[1].x, 500.0);
        let stacked = grid(2, AREA, true, 0.0);
        assert_eq!(stacked[0].w, AREA.w);
        assert_eq!(stacked[1].y, 340.0);
    }

    #[test]
    fn three_has_no_hole() {
        let r = grid(3, AREA, false, 0.0);
        assert_eq!(r[2].w, AREA.w);
        assert_eq!(covered(&r, 0.0), AREA.w * AREA.h);
    }

    #[test]
    fn columns_split_the_width() {
        let r = columns(3, AREA, 0.0);
        assert_eq!(r.len(), 3);
        assert!((r[2].x + r[2].w - AREA.w).abs() < 1e-9);
        assert!(r.iter().all(|c| c.h == AREA.h));
    }

    #[test]
    fn focus_shows_only_one() {
        let r = arrange(LayoutMode::Focus, 3, Some(1), AREA, false, 4.0);
        assert_eq!(r, vec![None, Some(AREA), None]);
    }

    #[test]
    fn four_is_two_by_two() {
        let r = grid(4, AREA, false, 0.0);
        assert_eq!(r.len(), 4);
        assert_eq!(covered(&r, 0.0), AREA.w * AREA.h);
    }
}
