use ipc::MASK_RECTS;

use crate::widgets::{TreeWidget, WidgetTree};

pub type Rect = [i16; 4];

const RECTANGLE: i32 = 3;
const GRAPHIC: i32 = 5;
const TRANSPARENT: i32 = 255;
const FULL_SCREEN_SHARE: f64 = 0.9;

pub fn interface_rects(tree: &WidgetTree, [width, height]: [i32; 2]) -> Vec<Rect> {
    let viewport = f64::from(width.max(0)) * f64::from(height.max(0));
    let rects = tree
        .widgets
        .iter()
        .filter(|tree_widget| opaque(tree_widget))
        .filter_map(|tree_widget| {
            let w = &tree_widget.widget;
            clip(
                [w.x, w.y, w.x.saturating_add(w.width), w.y.saturating_add(w.height)],
                [width, height],
            )
        })
        .filter(|&rect| (area(rect) as f64) < viewport * FULL_SCREEN_SHARE)
        .collect();
    simplify(rects)
}

fn opaque(TreeWidget { widget, detail, .. }: &TreeWidget) -> bool {
    let drawn = match widget.kind {
        GRAPHIC => widget.sprite >= 0,
        RECTANGLE => detail.filled,
        _ => false,
    };
    drawn && !widget.hidden && detail.opacity < TRANSPARENT
}

fn clip([x1, y1, x2, y2]: [i32; 4], [width, height]: [i32; 2]) -> Option<Rect> {
    let fit = |v: i32, max: i32| v.clamp(0, max.clamp(0, i32::from(i16::MAX))) as i16;
    let rect = [fit(x1, width), fit(y1, height), fit(x2, width), fit(y2, height)];
    (area(rect) > 0).then_some(rect)
}

fn area([x1, y1, x2, y2]: Rect) -> i64 {
    (i64::from(x2) - i64::from(x1)).max(0) * (i64::from(y2) - i64::from(y1)).max(0)
}

fn contains(outer: Rect, inner: Rect) -> bool {
    outer[0] <= inner[0] && outer[1] <= inner[1] && outer[2] >= inner[2] && outer[3] >= inner[3]
}

fn union(a: Rect, b: Rect) -> Rect {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn simplify(mut rects: Vec<Rect>) -> Vec<Rect> {
    rects.sort_by_key(|&rect| std::cmp::Reverse(area(rect)));
    let mut kept: Vec<Rect> = Vec::with_capacity(rects.len());
    for rect in rects {
        if !kept.iter().any(|&outer| contains(outer, rect)) {
            kept.push(rect);
        }
    }
    while kept.len() > MASK_RECTS {
        let mut best = (0, 1, i64::MAX);
        for i in 0..kept.len() {
            for j in i + 1..kept.len() {
                let grown = area(union(kept[i], kept[j])) - area(kept[i]) - area(kept[j]);
                if grown < best.2 {
                    best = (i, j, grown);
                }
            }
        }
        let (i, j, _) = best;
        let merged = union(kept[i], kept[j]);
        kept.swap_remove(j);
        kept[i] = merged;
        kept.retain(|&rect| rect == merged || !contains(merged, rect));
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::{Widget, WidgetDetail};

    const VIEWPORT: [i32; 2] = [1420, 1852];

    fn widget(kind: i32, [x, y, width, height]: [i32; 4], sprite: i32) -> TreeWidget {
        TreeWidget {
            widget: Widget {
                id: 0,
                group: 0,
                child: 0,
                kind,
                x,
                y,
                width,
                height,
                parent: -1,
                sprite,
                index: -1,
                hidden: false,
                text: None,
                text_color: -1,
                scroll_x: 0,
                scroll_y: 0,
                scroll_width: 0,
                scroll_height: 0,
                action: None,
                item_id: -1,
                item_quantity: 0,
            },
            detail: WidgetDetail::default(),
            dynamic: false,
            owner: -1,
        }
    }

    fn tree(widgets: Vec<TreeWidget>) -> WidgetTree {
        WidgetTree {
            root: 161,
            sub_interfaces: Vec::new(),
            widgets,
        }
    }

    fn sorted(mut rects: Vec<Rect>) -> Vec<Rect> {
        rects.sort();
        rects
    }

    #[test]
    fn sprites_and_filled_rectangles_mask_their_bounds() {
        let mut filled = widget(RECTANGLE, [947, 1335, 374, 447], -1);
        filled.detail.filled = true;
        let rects = interface_rects(
            &tree(vec![widget(GRAPHIC, [0, 1563, 908, 249], 1017), filled]),
            VIEWPORT,
        );
        assert_eq!(sorted(rects), [[0, 1563, 908, 1812], [947, 1335, 1321, 1782]]);
    }

    #[test]
    fn containers_text_outlines_and_invisible_widgets_mask_nothing() {
        let mut hidden = widget(GRAPHIC, [10, 10, 50, 50], 3);
        hidden.widget.hidden = true;
        let mut clear = widget(GRAPHIC, [100, 10, 50, 50], 3);
        clear.detail.opacity = TRANSPARENT;
        let widgets = vec![
            widget(0, [0, 0, 908, 1852], -1),
            widget(4, [0, 0, 300, 40], -1),
            widget(RECTANGLE, [200, 200, 100, 100], -1),
            widget(GRAPHIC, [400, 400, 30, 30], -1),
            hidden,
            clear,
        ];
        assert_eq!(interface_rects(&tree(widgets), VIEWPORT), Vec::<Rect>::new());
    }

    #[test]
    fn half_transparent_widgets_still_mask() {
        let mut counter = widget(GRAPHIC, [679, 0, 226, 50], 1176);
        counter.detail.opacity = 100;
        assert_eq!(interface_rects(&tree(vec![counter]), VIEWPORT), [[679, 0, 905, 50]]);
    }

    #[test]
    fn rects_are_clipped_and_full_screen_ones_skipped() {
        let widgets = vec![
            widget(GRAPHIC, [1321, -2, 120, 1854], 7),
            widget(GRAPHIC, [-10, -10, 1500, 1900], 8),
        ];
        assert_eq!(interface_rects(&tree(widgets), VIEWPORT), [[1321, 0, 1420, 1852]]);
    }

    #[test]
    fn widgets_inside_a_panel_are_dropped() {
        let widgets = vec![
            widget(GRAPHIC, [959, 1313, 350, 492], 1),
            widget(GRAPHIC, [1017, 1390, 234, 63], 2),
        ];
        assert_eq!(interface_rects(&tree(widgets), VIEWPORT), [[959, 1313, 1309, 1805]]);
    }

    #[test]
    fn more_than_the_limit_merges_the_closest_pairs() {
        let rects: Vec<Rect> = (0..MASK_RECTS as i16 + 3)
            .map(|i| [i * 20, 0, i * 20 + 10, 10])
            .collect();
        let simplified = simplify(rects.clone());
        assert_eq!(simplified.len(), MASK_RECTS);
        for rect in rects {
            assert!(simplified.iter().any(|&outer| contains(outer, rect)));
        }
    }
}
