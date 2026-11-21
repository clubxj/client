use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::{Theme, ThemeMode, ThemeTokens};
use gpui_kit::{App, FontFallbacks, Hsla, Pixels, Rgba, SharedString, px};

#[derive(Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UiFont {
    #[default]
    Proportional,
    Monospace,
}

impl UiFont {
    pub const ALL: [Self; 2] = [Self::Proportional, Self::Monospace];

    pub fn label(self) -> &'static str {
        match self {
            Self::Proportional => "Proportional",
            Self::Monospace => "Monospace",
        }
    }
}

const fn hex(value: u32) -> Rgba {
    Rgba {
        r: ((value >> 16) & 0xff) as f32 / 255.0,
        g: ((value >> 8) & 0xff) as f32 / 255.0,
        b: (value & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

pub const BG_DARK: Rgba = hex(0x16161e);
pub const BG: Rgba = hex(0x1a1b26);
pub const BG_FLOAT: Rgba = hex(0x1f2335);
pub const BG_HL: Rgba = hex(0x292e42);
pub const BORDER: Rgba = hex(0x3b4261);
pub const FG: Rgba = hex(0xc0caf5);
pub const FG_DARK: Rgba = hex(0xa9b1d6);
pub const COMMENT: Rgba = hex(0x565f89);
pub const BLUE: Rgba = hex(0x7aa2f7);
pub const SEL: Rgba = hex(0x2e3c64);
pub const GREEN: Rgba = hex(0x9ece6a);
pub const YELLOW: Rgba = hex(0xe0af68);
pub const PURPLE: Rgba = hex(0xbb9af7);
pub const RED: Rgba = hex(0xf7768e);
pub const CYAN: Rgba = hex(0x7dcfff);

pub const SANS: &str = "Noto Sans";
pub const MONO: &str = "FiraCode Nerd Font Mono";

pub const SMALL: Pixels = px(11.0);
pub const BODY: Pixels = px(13.0);
pub const HEADING: Pixels = px(16.0);
pub const ROW_HEIGHT: Pixels = px(28.0);
pub const INTERACT_HEIGHT: Pixels = px(22.0);
pub const ITEM_GAP: Pixels = px(8.0);
pub const LINE_GAP: Pixels = px(4.0);
pub const BUTTON_PADDING: Pixels = px(8.0);

pub fn alpha(color: Rgba, alpha: f32) -> Hsla {
    let mut color: Hsla = color.into();
    color.a = alpha;
    color
}

pub fn ui_family(ui_font: UiFont) -> SharedString {
    match ui_font {
        UiFont::Proportional => SANS.into(),
        UiFont::Monospace => MONO.into(),
    }
}

pub fn fallbacks() -> FontFallbacks {
    FontFallbacks::from_fonts(vec![MONO.to_owned()])
}

pub fn install(cx: &mut App, ui_font: UiFont) {
    cx.text_system()
        .add_fonts(vec![
            Cow::Borrowed(&include_bytes!("../../assets/FiraCodeNerdFontMono-Regular.ttf")[..]),
            Cow::Borrowed(&include_bytes!("../../assets/FiraCodeNerdFontMono-Bold.ttf")[..]),
            Cow::Borrowed(&include_bytes!("../../assets/NotoSans-Regular.ttf")[..]),
            Cow::Borrowed(&include_bytes!("../../assets/NotoSans-Bold.ttf")[..]),
        ])
        .expect("the bundled fonts load");
    Theme::change(ThemeMode::Dark, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_size = BODY;
    theme.mono_font_family = MONO.into();
    theme.mono_font_size = BODY;
    theme.radius = px(0.0);
    theme.radius_lg = px(0.0);
    theme.window_radius = px(0.0);
    theme.shadow = false;
    let c = &mut theme.colors;
    c.background = BG.into();
    c.foreground = FG.into();
    c.border = BORDER.into();
    c.input = BORDER.into();
    c.ring = BLUE.into();
    c.caret = FG.into();
    c.selection = SEL.into();
    c.muted = BG_HL.into();
    c.muted_foreground = COMMENT.into();
    c.accent = SEL.into();
    c.accent_foreground = FG.into();
    c.popover = BG_FLOAT.into();
    c.popover_foreground = FG.into();
    c.overlay = alpha(BG_DARK, 0.5);
    c.link = BLUE.into();
    c.link_hover = BLUE.into();
    c.link_active = BLUE.into();
    c.primary = BG_HL.into();
    c.primary_hover = SEL.into();
    c.primary_active = SEL.into();
    c.primary_foreground = FG.into();
    c.secondary = BG_HL.into();
    c.secondary_hover = SEL.into();
    c.secondary_active = SEL.into();
    c.secondary_foreground = FG.into();
    c.button = BG_HL.into();
    c.button_hover = SEL.into();
    c.button_active = SEL.into();
    c.button_foreground = FG.into();
    c.button_primary = BLUE.into();
    c.button_primary_hover = BLUE.into();
    c.button_primary_active = BLUE.into();
    c.button_primary_foreground = BG_DARK.into();
    c.button_secondary = BG_HL.into();
    c.button_secondary_hover = SEL.into();
    c.button_secondary_active = SEL.into();
    c.button_secondary_foreground = FG.into();
    c.danger = RED.into();
    c.danger_hover = RED.into();
    c.danger_active = RED.into();
    c.danger_foreground = BG_DARK.into();
    c.warning = YELLOW.into();
    c.warning_hover = YELLOW.into();
    c.warning_active = YELLOW.into();
    c.warning_foreground = BG_DARK.into();
    c.success = GREEN.into();
    c.success_hover = GREEN.into();
    c.success_active = GREEN.into();
    c.success_foreground = BG_DARK.into();
    c.info = CYAN.into();
    c.info_hover = CYAN.into();
    c.info_active = CYAN.into();
    c.info_foreground = BG_DARK.into();
    c.list = BG.into();
    c.list_even = BG.into();
    c.list_head = BG_FLOAT.into();
    c.list_hover = BG_HL.into();
    c.list_active = SEL.into();
    c.list_active_border = BLUE.into();
    c.table = BG.into();
    c.table_even = BG_FLOAT.into();
    c.table_head = BG_FLOAT.into();
    c.table_head_foreground = FG.into();
    c.table_hover = BG_HL.into();
    c.table_active = SEL.into();
    c.table_active_border = BLUE.into();
    c.table_row_border = BG_HL.into();
    c.tab = BG.into();
    c.tab_bar = BG.into();
    c.tab_active = BG_HL.into();
    c.tab_active_foreground = FG.into();
    c.tab_foreground = COMMENT.into();
    c.scrollbar = alpha(BG_DARK, 0.0);
    c.scrollbar_thumb = BORDER.into();
    c.scrollbar_thumb_hover = COMMENT.into();
    c.sidebar = BG_DARK.into();
    c.sidebar_foreground = FG_DARK.into();
    c.sidebar_border = BG_DARK.into();
    c.sidebar_accent = BG_HL.into();
    c.sidebar_accent_foreground = BLUE.into();
    c.sidebar_primary = BLUE.into();
    c.sidebar_primary_foreground = BG_DARK.into();
    c.switch = BG_HL.into();
    c.switch_thumb = FG.into();
    c.slider_bar = BLUE.into();
    c.slider_thumb = FG.into();
    c.progress_bar = BLUE.into();
    c.window_border = BORDER.into();
    c.title_bar = BG.into();
    c.title_bar_border = BORDER.into();
    c.red = RED.into();
    c.green = GREEN.into();
    c.blue = BLUE.into();
    c.yellow = YELLOW.into();
    c.magenta = PURPLE.into();
    c.cyan = CYAN.into();
    theme.tokens = ThemeTokens::from(&theme.colors);
    set_ui_font(cx, ui_font);
}

static CURRENT_UI_FONT: AtomicBool = AtomicBool::new(false);

pub fn current_ui_family() -> SharedString {
    ui_family(if CURRENT_UI_FONT.load(Ordering::Relaxed) {
        UiFont::Monospace
    } else {
        UiFont::Proportional
    })
}

pub fn set_ui_font(cx: &mut App, ui_font: UiFont) {
    CURRENT_UI_FONT.store(ui_font == UiFont::Monospace, Ordering::Relaxed);
    Theme::global_mut(cx).font_family = ui_family(ui_font);
    cx.refresh_windows();
}
