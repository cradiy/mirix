//! Font-shaped measurements and bounded candidate pagination.
use gpui::{FontWeight, Pixels, Point, SharedString, Size, TextRun, Window, point, px, size};
use mirix_core::PanelSnapshot;
use mirix_skin::{LayoutStyle, Skin};
use std::ops::Range;

pub(super) const INLINE_GAP: f32 = 10.0;
pub(super) const NAV_HEIGHT: f32 = 34.0;
pub(super) const NAV_WIDTH: f32 = 68.0;

#[derive(Clone, Debug)]
pub(super) struct CandidateMeasure {
    pub width: f32,
    pub height: f32,
    pub text_width: f32,
    pub stacked: bool,
    pub annotation: Option<SharedString>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct PanelLayout {
    pub size: Size<Pixels>,
    pub content_size: Size<Pixels>,
    pub page: Range<usize>,
    pub ends: Vec<usize>,
    pub items: Vec<CandidateMeasure>,
    pub positions: Vec<Point<Pixels>>,
    pub label_width: f32,
    pub header_height: f32,
    pub top_padding: f32,
    pub navigation: bool,
    pub vertical: bool,
    pub inline_navigation: bool,
    pages: Vec<Page>,
    chrome: (f32, f32),
    inner_width: f32,
    pub(super) preedit_width: f32,
    gap: f32,
}

#[derive(Clone, Debug)]
struct Page {
    range: Range<usize>,
    positions: Vec<Point<Pixels>>,
    width: f32,
    height: f32,
}

pub(super) fn calculate(
    snapshot: &PanelSnapshot,
    skin: &Skin,
    show_preedit: bool,
    available: Size<Pixels>,
    window: &Window,
) -> PanelLayout {
    let show_preedit = snapshot.show_panel_preedit(show_preedit && skin.layout.show_preedit);
    let has_header = show_preedit && !snapshot.panel_text().is_empty();
    let top_padding = skin.canvas.content_top
        + if has_header {
            skin.preedit.top_padding
        } else {
            skin.layout.padding
        };
    let chrome_x = skin.canvas.content_left
        + skin.canvas.content_right
        + 2.0 * (skin.layout.padding + skin.window.border_width);
    let chrome_y = top_padding
        + skin.canvas.content_bottom
        + skin.layout.padding
        + 2.0 * skin.window.border_width;
    let max_width = skin
        .layout
        .max_width
        .min(f32::from(available.width))
        .max(chrome_x + 40.0);
    let max_height = skin
        .layout
        .max_height
        .min(f32::from(available.height))
        .max(chrome_y + 60.0);
    let label_size = (skin.candidate.font_size - 2.0).max(10.0);
    let label_width = if skin.layout.show_labels {
        (1..=9)
            .map(|n| measure(window, &n.to_string(), label_size, FontWeight::MEDIUM))
            .fold(0.0, f32::max)
    } else {
        0.0
    };
    let label_space = if skin.layout.show_labels {
        label_width + INLINE_GAP
    } else {
        0.0
    };
    let inner_width = (max_width - chrome_x).max(40.0);
    let header_height = if has_header {
        skin.preedit.font_size * 1.5 + skin.preedit.padding_y * 2.0 + 1.0 + skin.layout.gap
    } else {
        0.0
    };
    let widths: Vec<_> = snapshot
        .candidates
        .iter()
        .map(|candidate| {
            let text = measure(
                window,
                &candidate.text,
                skin.candidate.font_size,
                FontWeight::NORMAL,
            );
            let annotation = candidate_annotation(
                candidate.reading.as_deref(),
                candidate.comment.as_deref(),
                snapshot.panel_text(),
                show_preedit,
                skin,
            );
            let comment = annotation
                .as_deref()
                .map(|text| measure(window, text, label_size, FontWeight::NORMAL))
                .unwrap_or(0.0);
            (text, comment, annotation)
        })
        .collect();
    let first_widths: f32 = widths
        .iter()
        .take(3)
        .map(|(text, comment, _)| {
            text + comment
                + if *comment > 0.0 { INLINE_GAP } else { 0.0 }
                + label_space
                + skin.candidate.padding_x * 2.0
                + skin.layout.gap
        })
        .sum();
    // Decide from the leading candidates only. A long result on a later page
    // is wrapped by its own measurement and must not rotate the whole popup.
    let style = match skin.layout.style {
        LayoutStyle::Auto if first_widths > inner_width => LayoutStyle::Vertical,
        LayoutStyle::Auto | LayoutStyle::Compact => LayoutStyle::Horizontal,
        style => style,
    };
    let vertical = style == LayoutStyle::Vertical;
    let inline_navigation = style == LayoutStyle::Horizontal;
    let text_limit = (inner_width - label_space - skin.candidate.padding_x * 2.0).max(16.0);
    let text_limit = if inline_navigation {
        (text_limit - NAV_WIDTH - skin.layout.gap).max(16.0)
    } else {
        text_limit
    };
    let preedit_width = if header_height > 0.0 {
        measure(
            window,
            snapshot.panel_text(),
            skin.preedit.font_size,
            FontWeight::NORMAL,
        ) + skin.preedit.padding_x * 2.0
    } else {
        0.0
    };
    let min_vertical_text = (skin.layout.vertical_min_width - chrome_x).max(preedit_width)
        - label_space
        - skin.candidate.padding_x * 2.0;
    let vertical_width = widths
        .iter()
        .map(|(text, comment, _)| text.max(*comment))
        .fold(0.0, f32::max)
        .max(min_vertical_text)
        .max(40.0)
        .min(text_limit);
    let items: Vec<_> = widths
        .iter()
        .enumerate()
        .map(|(index, (text, comment, annotation))| {
            let (text, comment) = (*text, *comment);
            let stacked = vertical || text + comment + INLINE_GAP > text_limit;
            let text_width = if vertical {
                vertical_width
            } else if stacked {
                text.max(comment).min(text_limit)
            } else {
                text + comment + if comment > 0.0 { INLINE_GAP } else { 0.0 }
            }
            .max(1.0);
            let text_width = fitted_text_width(text_width, text_limit, window.scale_factor());
            let candidate = &snapshot.candidates[index];
            let text_height = wrapped_height(
                window,
                &candidate.text,
                skin.candidate.font_size,
                text_width,
            );
            let height = if stacked && comment > 0.0 {
                text_height
                    + 2.0
                    + wrapped_height(
                        window,
                        annotation.as_deref().unwrap_or(""),
                        label_size,
                        text_width,
                    )
            } else {
                text_height
            };
            CandidateMeasure {
                width: text_width + label_space + skin.candidate.padding_x * 2.0,
                height: height + skin.candidate.padding_y * 2.0,
                text_width,
                stacked,
                annotation: annotation.clone(),
            }
        })
        .collect();
    // A single page needs no controls. Reserve their space only when the
    // content really needs pagination, then repack before drawing anything.
    let limit_height = (max_height - chrome_y - header_height).max(24.0);
    let pages = paginate(
        &items,
        style,
        inner_width,
        limit_height,
        skin.layout.gap,
        skin.layout.max_candidates,
    );
    let ends = pages.iter().map(|page| page.range.end).collect::<Vec<_>>();
    let navigation = pages.len() > 1;
    let mut layout = PanelLayout {
        ends,
        items,
        label_width,
        header_height,
        top_padding,
        navigation,
        vertical,
        inline_navigation,
        pages,
        chrome: (chrome_x, chrome_y),
        inner_width,
        preedit_width,
        gap: skin.layout.gap,
        ..Default::default()
    };
    layout.select(snapshot.selected as usize);
    layout
}

impl PanelLayout {
    /// Paging reuses all shaped text and measured pages until their inputs change.
    pub(super) fn select(&mut self, selected: usize) {
        let selected = selected.min(self.items.len().saturating_sub(1));
        let current = self
            .pages
            .iter()
            .find(|page| page.range.contains(&selected));
        let navigation_width = if self.navigation && self.inline_navigation {
            NAV_WIDTH + self.gap
        } else {
            0.0
        };
        let content_width = current
            .map_or(0.0, |page| page.width + navigation_width)
            .max(self.preedit_width.min(self.inner_width))
            .max(if self.navigation { 100.0 } else { 40.0 })
            .min(self.inner_width);
        let content_height = current.map_or(0.0, |page| page.height).max(
            if self.navigation && self.inline_navigation {
                NAV_HEIGHT
            } else {
                0.0
            },
        );
        let height = self.chrome.1
            + self.header_height
            + content_height
            + if self.navigation && !self.inline_navigation {
                NAV_HEIGHT + 8.0
            } else {
                0.0
            };
        self.size = size(
            px((content_width + self.chrome.0).ceil()),
            px(height.ceil()),
        );
        self.content_size = size(
            px(current.map_or(0.0, |page| page.width).ceil()),
            px(content_height.ceil()),
        );
        self.page = current.map_or(0..0, |page| page.range.clone());
        if let Some(page) = current {
            self.positions.clone_from(&page.positions);
        } else {
            self.positions.clear();
        }
    }
}

fn candidate_annotation(
    reading: Option<&str>,
    comment: Option<&str>,
    preedit: &str,
    show_preedit: bool,
    skin: &Skin,
) -> Option<SharedString> {
    reading
        .filter(|text| !text.is_empty())
        .map(|text| format!("[{text}]").into())
        .or_else(|| {
            comment
                .filter(|text| {
                    skin.layout.show_comments
                        && !text.is_empty()
                        && (show_preedit || *text != preedit)
                })
                .map(|text| text.to_owned().into())
        })
}

pub(super) fn measure(window: &Window, text: &str, font_size: f32, weight: FontWeight) -> f32 {
    let mut font = window.text_style().font();
    font.weight = weight;
    let run = TextRun {
        len: text.len(),
        font,
        ..Default::default()
    };
    f32::from(
        window
            .text_system()
            .shape_line(
                SharedString::from(text.to_owned()),
                px(font_size),
                &[run],
                None,
            )
            .width(),
    )
}

fn fitted_text_width(natural: f32, limit: f32, scale: f32) -> f32 {
    // GPUI aligns explicit widths to device pixels. Rounding a tightly fitted
    // width down can wrap the last glyph even though measurement fit one line.
    // Use the same aligned width for shaping and drawing, without exceeding
    // the available space when a long candidate really needs to wrap.
    (natural * scale)
        .ceil()
        .min((limit * scale).floor())
        .max(1.0)
        / scale
}

fn wrapped_height(window: &Window, text: &str, font_size: f32, width: f32) -> f32 {
    let line_height = window.pixel_snap(px(font_size * 1.5));
    let mut font = window.text_style().font();
    font.weight = FontWeight::NORMAL;
    let run = TextRun {
        len: text.len(),
        font,
        ..Default::default()
    };
    window
        .text_system()
        .shape_text(
            text.to_owned().into(),
            px(font_size),
            &[run],
            Some(px(width)),
            None,
        )
        .map(|lines| {
            lines
                .iter()
                .map(|line| f32::from(line.size(line_height).height))
                .sum::<f32>()
        })
        .unwrap_or(f32::from(line_height))
        .max(f32::from(line_height))
}

fn paginate(
    items: &[CandidateMeasure],
    style: LayoutStyle,
    width: f32,
    height: f32,
    gap: f32,
    max_candidates: usize,
) -> Vec<Page> {
    let pages = pack(items, style, width, height, gap, max_candidates);
    if pages.len() <= 1 {
        return pages;
    }
    if style == LayoutStyle::Horizontal {
        pack(
            items,
            style,
            (width - NAV_WIDTH - gap).max(1.0),
            height,
            gap,
            max_candidates,
        )
    } else {
        pack(
            items,
            style,
            width,
            (height - NAV_HEIGHT - 8.0).max(24.0),
            gap,
            max_candidates,
        )
    }
}

fn pack(
    items: &[CandidateMeasure],
    style: LayoutStyle,
    width: f32,
    height: f32,
    gap: f32,
    max_candidates: usize,
) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let mut page = Page {
        range: 0..0,
        positions: Vec::new(),
        width: 0.0,
        height: 0.0,
    };
    let mut next_x = 0.0;
    let mut row_y = 0.0;
    for (index, item) in items.iter().enumerate() {
        let item_width = item.width.min(width);
        let item_height = item.height.min(height);
        let (mut x, mut y) = match style {
            LayoutStyle::Vertical => (
                0.0,
                if page.positions.is_empty() {
                    0.0
                } else {
                    page.height + gap
                },
            ),
            LayoutStyle::Grid if next_x + item_width > width => (0.0, page.height + gap),
            _ => (next_x, row_y),
        };
        if !page.positions.is_empty()
            && (page.positions.len() >= max_candidates
                || x + item_width > width + 0.01
                || y + item_height > height + 0.01)
        {
            pages.push(page);
            page = Page {
                range: index..index,
                positions: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
            x = 0.0;
            y = 0.0;
        }
        page.positions.push(point(px(x), px(y)));
        page.width = page.width.max(x + item_width);
        page.height = page.height.max(y + item_height);
        page.range.end = index + 1;
        next_x = x + item_width + gap;
        row_y = y;
    }
    if !page.positions.is_empty() {
        pages.push(page);
    }
    pages
}

#[cfg(test)]
mod tests {
    #[test]
    fn reading_is_not_hidden_by_skin_or_duplicate_preedit() {
        let skin = mirix_skin::Skin::default();
        assert_eq!(
            super::candidate_annotation(Some("きみ"), None, "きみ", false, &skin).as_deref(),
            Some("[きみ]")
        );
        assert_eq!(
            super::candidate_annotation(None, Some("note"), "", false, &skin),
            None
        );
    }

    #[test]
    fn brackets_apply_only_to_readings_and_empty_readings_allow_comments() {
        let mut skin = mirix_skin::Skin::default();
        skin.layout.show_comments = true;
        for (reading, comment, expected) in [
            (Some("nǐ hǎo"), Some("greeting"), Some("[nǐ hǎo]")),
            (Some(""), Some("greeting"), Some("greeting")),
            (None, Some("greeting"), Some("greeting")),
            (Some(""), None, None),
        ] {
            assert_eq!(
                super::candidate_annotation(reading, comment, "", false, &skin).as_deref(),
                expected,
            );
        }
    }
    use super::*;
    #[test]
    fn fractional_scale_does_not_wrap_fitting_candidates() {
        let platform = gpui_platform::current_platform(true);
        let text_system = gpui::WindowTextSystem::new(std::sync::Arc::new(gpui::TextSystem::new(
            platform.text_system(),
        )));
        for scale in [1.0_f32, 1.25, 1.5, 1.75, 2.0] {
            for count in 12..=36 {
                let text: SharedString = "候".repeat(count).into();
                let runs = [TextRun {
                    len: text.len(),
                    font: gpui::font(".SystemUIFont"),
                    ..Default::default()
                }];
                let natural = f32::from(
                    text_system
                        .shape_line(text.clone(), px(17.0), &runs, None)
                        .width(),
                );
                let fitted = fitted_text_width(natural, 700.0, scale);
                // GPUI's layout rounds positive lengths to the nearest device
                // pixel, with half-pixel ties rounded down.
                let rendered = (fitted * scale - 0.5).ceil() / scale;
                let lines = text_system
                    .shape_text(text, px(17.0), &runs, Some(px(rendered)), None)
                    .unwrap();
                let height: f32 = lines
                    .iter()
                    .map(|line| f32::from(line.size(px(26.0)).height))
                    .sum();
                assert_eq!(
                    height, 26.0,
                    "scale={scale} characters={count} natural={natural} rendered={rendered}"
                );
            }
        }
    }

    #[test]
    fn capped_candidate_has_the_same_wrapped_height_after_pixel_alignment() {
        let platform = gpui_platform::current_platform(true);
        let text_system = gpui::WindowTextSystem::new(std::sync::Arc::new(gpui::TextSystem::new(
            platform.text_system(),
        )));
        let text: SharedString = "候".repeat(40).into();
        let runs = [TextRun {
            len: text.len(),
            font: gpui::font(".SystemUIFont"),
            ..Default::default()
        }];
        let height = |width| {
            text_system
                .shape_text(text.clone(), px(17.0), &runs, Some(px(width)), None)
                .unwrap()
                .iter()
                .map(|line| f32::from(line.size(px(26.0)).height))
                .sum::<f32>()
        };
        for scale in [1.0_f32, 1.25, 1.5, 1.75, 2.0] {
            for limit in [220.5, 221.0, 221.5, 222.0] {
                let fitted = fitted_text_width(680.0, limit, scale);
                let rendered = (fitted * scale - 0.5).ceil() / scale;
                assert!(fitted <= limit);
                assert!(height(fitted) > 26.0);
                assert_eq!(
                    height(fitted),
                    height(rendered),
                    "scale={scale} limit={limit}"
                );
            }
        }
    }

    fn item(width: f32, height: f32) -> CandidateMeasure {
        CandidateMeasure {
            width,
            height,
            text_width: width,
            stacked: false,
            annotation: None,
        }
    }

    #[test]
    fn horizontal_pages_follow_measured_word_widths() {
        let items = [
            item(80.0, 30.0),
            item(180.0, 30.0),
            item(60.0, 30.0),
            item(60.0, 30.0),
        ];
        let pages = pack(&items, LayoutStyle::Horizontal, 280.0, 200.0, 6.0, 9);
        assert_eq!(
            pages
                .iter()
                .map(|page| page.range.clone())
                .collect::<Vec<_>>(),
            [0..2, 2..4]
        );
        assert_eq!(pages[0].width, 266.0);
        assert_eq!(pages[1].width, 126.0);
    }

    #[test]
    fn vertical_pages_follow_row_heights_and_number_key_limit() {
        let items = vec![item(180.0, 40.0); 12];
        let pages = pack(&items, LayoutStyle::Vertical, 300.0, 140.0, 5.0, 9);
        assert_eq!(pages[0].range, 0..3);
        assert_eq!(pages[0].height, 130.0);
        assert_eq!(pages[1].range, 3..6);
        let pages = pack(&items, LayoutStyle::Vertical, 300.0, 1000.0, 5.0, 9);
        assert_eq!(pages[0].range, 0..9);
    }

    #[test]
    fn inline_controls_reserve_width_not_an_extra_row() {
        let items = vec![item(80.0, 45.0); 10];
        let pages = paginate(&items, LayoutStyle::Horizontal, 340.0, 80.0, 4.0, 5);
        assert_eq!(pages[0].range, 0..3);
        assert_eq!(pages[0].height, 45.0);
        assert!(pages[0].width + NAV_WIDTH + 4.0 <= 340.0);
    }

    #[test]
    fn single_page_does_not_reserve_navigation_space() {
        let items = vec![item(80.0, 45.0); 4];
        let pages = paginate(&items, LayoutStyle::Horizontal, 340.0, 80.0, 4.0, 5);
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].range, 0..4);
    }

    #[test]
    fn vertical_controls_reserve_bottom_space_and_skin_limits_count() {
        let items = vec![item(240.0, 45.0); 12];
        let pages = paginate(&items, LayoutStyle::Vertical, 240.0, 300.0, 4.0, 5);
        assert_eq!(pages[0].range, 0..5);
        assert!(pages[0].height + NAV_HEIGHT + 8.0 <= 300.0);
    }

    #[test]
    fn grid_wraps_rows_and_keeps_every_candidate() {
        let items = vec![item(100.0, 30.0); 7];
        let pages = pack(&items, LayoutStyle::Grid, 210.0, 70.0, 5.0, 9);
        assert_eq!(pages[0].range, 0..4);
        assert_eq!(pages[1].range, 4..7);
        assert_eq!(pages[0].positions[2], point(px(0.0), px(35.0)));
    }
}
