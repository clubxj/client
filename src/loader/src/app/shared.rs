use super::*;

pub(super) fn divider() -> Div {
    div()
        .flex_none()
        .h(px(1.0))
        .w_full()
        .bg(crate::ui::theme::BORDER)
}

pub(super) fn fit_row(fits: bool) -> Div {
    let row = div().flex().gap(crate::ui::theme::ITEM_GAP);
    if fits {
        row.items_center()
    } else {
        row.flex_col().gap(crate::ui::theme::LINE_GAP)
    }
}

pub(super) fn error_label(error: impl Into<gpui_kit::SharedString>) -> Div {
    crate::ui::widgets::error(error)
}
