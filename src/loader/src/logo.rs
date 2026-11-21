use gpui_kit::{
    AnyElement, App, Bounds, Hsla, IntoElement, Pixels, Rgba, SharedString, Styled, TextAlign,
    TextRun, Window, canvas, font, point, px,
};

use crate::ui::theme::{self, BLUE, CYAN, FG, PURPLE, RED, YELLOW};

const FLAME: [&str; 34] = [
    "                      .=-",
    "                    .+%@",
    "                   =%#@@",
    "                 .*###@@",
    "                 *####@@",
    "                =#####@@",
    "                ######@@@",
    "               .%#####@@@@",
    "               .######@@@@@",
    "           .    *######@@@@@.",
    "          .*#+-.:%#######@@@@@",
    "           =###%+*########@@@@@",
    "           -###############@@@@@",
    "           *################@@@@@   ..",
    "          +#################@@@@@   :#+:",
    "        .*##################@@@@@@  .##%+.",
    "       -###################@@@@@@@   #####.",
    "     .+###################@@@@@@@   .%#####.",
    "    :###################@@@@@@@@    =######+",
    "   =%################@@@@@@@@@@    :%######%",
    "  +################@@@@@@@@@@    .+%#######%",
    " +##############@@@@@@@@@@@   .-*%#########+",
    "-##############@@@@@@@@@@  :=*%############.",
    "##############@@@@@@@@@  -*##############%-",
    "%#############@@@@@@@@  .-+@############%-",
    "%#############@@@@@@@      @@@##########:",
    "*#############@@@@@@@@.    @@@#######%=",
    ":##############@@@@@@@@@@@@@@@######*:",
    " -%#############@@@@@@@@@@@@######*:",
    "  :#################@@@@#######%+:",
    "    =#########################+.",
    "     .-*%#####################=.",
    "        .-+*%##################=",
    "            .:=+*##%####%%#*+-.",
];

const WORDMARK: [(Rgba, &str); 6] = [
    (
        BLUE,
        "██╗   ██╗ ██████╗ ██╗      ██████╗██████╗ ███████╗███╗   ██╗",
    ),
    (
        BLUE,
        "██║   ██║██╔═══██╗██║     ██╔════╝██╔══██╗██╔════╝████╗  ██║",
    ),
    (
        CYAN,
        "██║   ██║██║   ██║██║     ██║     ██████╔╝█████╗  ██╔██╗ ██║",
    ),
    (
        CYAN,
        "╚██╗ ██╔╝██║   ██║██║     ██║     ██╔══██╗██╔══╝  ██║╚██╗██║",
    ),
    (
        PURPLE,
        " ╚████╔╝ ╚██████╔╝███████╗╚██████╗██║  ██║███████╗██║ ╚████║",
    ),
    (
        PURPLE,
        "  ╚═══╝   ╚═════╝ ╚══════╝ ╚═════╝╚═╝  ╚═╝╚══════╝╚═╝  ╚═══╝",
    ),
];

const ORANGE: Rgba = Rgba {
    r: 0xff as f32 / 255.0,
    g: 0x9e as f32 / 255.0,
    b: 0x64 as f32 / 255.0,
    a: 1.0,
};

const STRENGTH: f32 = 0.14;
const GLYPH_WIDTH: f32 = 0.6;

fn jitter(i: usize, j: usize) -> f64 {
    let x = (i as f64 * 12.9898 + j as f64 * 78.233).sin() * 43758.5453;
    (x - x.floor()) * 2.0 - 1.0
}

fn outer_color(i: usize, j: usize, width: usize, bias: f64) -> Rgba {
    let skew = (0.5 - j as f64 / width as f64) * 0.25;
    let t = (i as f64 + 0.5) / FLAME.len() as f64 + skew + bias * 0.07;
    [YELLOW, ORANGE, RED][((t * 3.0).floor().clamp(0.0, 2.0)) as usize]
}

fn inner_color(i: usize, j: usize, bias: f64) -> Rgba {
    let row = FLAME[i].as_bytes();
    let mut d = 0;
    while j > d && row[j - d - 1] == b'@' {
        d += 1;
    }
    if FLAME.get(i + 1).and_then(|next| next.as_bytes().get(j)) != Some(&b'@') {
        d = 0;
    }
    let edge = (1.0 - d as f64 / 4.0).max(0.0);
    let amount = ((i as f64 + 0.5) / FLAME.len() as f64 - 0.25) * 1.6 * edge + bias * 0.3;
    let pct = (amount.clamp(0.0, 0.9) * 5.0).round() / 5.0;
    lerp(FG, YELLOW, pct as f32)
}

fn lerp(from: Rgba, to: Rgba, t: f32) -> Rgba {
    let mix = |a: f32, b: f32| a + (b - a) * t;
    Rgba {
        r: mix(from.r, to.r),
        g: mix(from.g, to.g),
        b: mix(from.b, to.b),
        a: 1.0,
    }
}

const FONT_SIZE: f32 = 9.0;

type Line = (String, Vec<(usize, Rgba)>);

fn lines(columns: usize) -> Vec<Line> {
    let width = FLAME.iter().map(|line| line.len()).max().unwrap_or(1);
    let indent = " ".repeat(columns.saturating_sub(width) / 2);
    let mut lines: Vec<Line> = FLAME
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let mut runs: Vec<(usize, Rgba)> = vec![(indent.len(), FG)];
            for (j, c) in line.char_indices() {
                let bias = jitter(i, j);
                let color = if c == '@' {
                    inner_color(i, j, bias)
                } else {
                    outer_color(i, j, width, bias)
                };
                match runs.last_mut() {
                    Some((len, last)) if *last == color => *len += c.len_utf8(),
                    _ => runs.push((c.len_utf8(), color)),
                }
            }
            (format!("{indent}{line}"), runs)
        })
        .collect();
    lines.push((" ".to_owned(), vec![(1, FG)]));
    lines.extend(
        WORDMARK
            .iter()
            .map(|(color, line)| (line.to_string(), vec![(line.len(), *color)])),
    );
    lines
}

fn paint(bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
    let columns = WORDMARK
        .iter()
        .map(|(_, line)| line.chars().count())
        .max()
        .unwrap_or(0);
    let size =
        px(FONT_SIZE.min(f32::from(bounds.size.width) * 0.92 / (columns as f32 * GLYPH_WIDTH)));
    if size < px(5.0) {
        return;
    }
    let mono = font(theme::MONO);
    let shaped: Vec<_> = lines(columns)
        .into_iter()
        .map(|(text, runs)| {
            let runs: Vec<TextRun> = runs
                .into_iter()
                .filter(|(len, _)| *len > 0)
                .map(|(len, color)| {
                    let mut color: Hsla = color.into();
                    color.a = STRENGTH;
                    TextRun {
                        len,
                        font: mono.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }
                })
                .collect();
            window
                .text_system()
                .shape_line(SharedString::from(text), size, &runs, None)
        })
        .collect();
    let width = shaped
        .iter()
        .map(|line| line.width)
        .fold(px(0.0), Pixels::max);
    let height = size * shaped.len() as f32;
    let origin = bounds.center() - point(width / 2.0, height / 2.0);
    window.with_content_mask(Some(gpui_kit::ContentMask { bounds }), |window| {
        for (row, line) in shaped.iter().enumerate() {
            let at = origin + point(px(0.0), size * row as f32);
            line.paint(at, size, TextAlign::Left, None, window, cx).ok();
        }
    });
}

pub fn background() -> AnyElement {
    canvas(
        |_, _, _| {},
        |bounds, (), window, cx| paint(bounds, window, cx),
    )
    .absolute()
    .inset_0()
    .into_any_element()
}
