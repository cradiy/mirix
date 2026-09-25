//! Rendering uses the measured boxes verbatim: candidates never flex-shrink.
use super::client::navigate_candidates;
use super::effects;
use super::layers::SkinScene;
use super::layout::{INLINE_GAP, NAV_HEIGHT, NAV_WIDTH};
use super::view::CandidatePanel;
use gpui::{
    Background, BoxShadow, Context as GpuiContext, FontWeight, PathBuilder, Window,
    border_color_stop, border_gradient, canvas, div, linear_color_stop, linear_gradient, point,
    prelude::*, px, rgba,
};
use mirix_core::{PunctuationMode, keysym};
use mirix_skin::{LayerPlane, MotionStyle, WindowStyle};
use std::time::Instant;

impl Render for CandidatePanel {
    fn render(&mut self, window: &mut Window, cx: &mut GpuiContext<Self>) -> impl IntoElement {
        let animations_enabled = self.animations_enabled;
        self.update_window_size(window, cx);
        let now = Instant::now();
        let visible = self.visible();
        self.motion
            .sync(&self.snapshot, self.layout.page.start, visible, now);
        let body_bounds = self.body_bounds();
        let mut surface = div()
            .relative()
            .overflow_hidden()
            .w(self.size.width)
            .h(self.size.height);
        if !visible {
            return surface.into_any_element();
        }
        let show_progress = if !animations_enabled || self.skin.transitions.duration_ms == 0 {
            1.0
        } else {
            (self.motion.age_ms(now) / self.skin.transitions.duration_ms as f32).min(1.0)
        };
        let animating = animations_enabled
            && (self.motion.running(&self.skin, now)
                || (matches!(self.skin.transitions.show, MotionStyle::Fade)
                    && show_progress < 1.0));
        self.report_rebuilt_show(window);
        if animating {
            window.request_animation_frame();
        }
        let mut root = div()
            .absolute()
            .left(body_bounds.origin.x)
            .top(body_bounds.origin.y)
            .overflow_hidden()
            .w(body_bounds.size.width)
            .h(body_bounds.size.height)
            .font_weight(FontWeight::NORMAL);
        let skin = &self.skin;
        let images = if self.skin_root.is_some() && !skin.layers.is_empty() {
            Some(
                self.skin_images
                    .get_or_insert_with(|| gpui::RetainAllImageCache::new(cx)),
            )
        } else {
            None
        };
        let scene = self
            .skin_root
            .as_ref()
            .zip(images)
            .map(|(path, images)| SkinScene {
                skin,
                root: path,
                images,
                preedit: self.snapshot.panel_text(),
                has_candidates: !self.snapshot.candidates.is_empty(),
                bounds: gpui::Bounds {
                    origin: point(px(0.0), px(0.0)),
                    size: body_bounds.size,
                },
                motion: &self.motion,
                animations_enabled,
                now,
            });
        if let Some(scene) = &scene {
            root = root.children(scene.render(LayerPlane::Background));
        }
        // Particles belong inside the rounded panel, beneath its interactive text.
        if animations_enabled && let Some(particles) = &skin.particles {
            root = root.child(effects::particles(particles, self.motion.age_ms(now)));
        }
        let context_id = self.snapshot.context_id.map(|id| id.0);
        let commands = self.services.commands();
        let page = self.layout.page.clone();
        let selected = self.snapshot.selected as usize;
        let selection_motion = if animations_enabled {
            self.motion.values(skin, "selection", now)
        } else {
            Default::default()
        };
        let selection_background = skin
            .candidate
            .selected_background_alt
            .map(|alternate| {
                linear_gradient(
                    110.0,
                    linear_color_stop(rgba(skin.candidate.selected_background.0), 0.0),
                    linear_color_stop(rgba(alternate.0), 1.0),
                )
            })
            .unwrap_or_else(|| Background::from(rgba(skin.candidate.selected_background.0)));
        let mut list = div()
            .relative()
            .flex_shrink_0()
            .rounded(px(skin.candidate.radius))
            .bg(rgba(skin.candidate.strip_background.0))
            .w(self.layout.content_size.width)
            .h(self.layout.content_size.height);
        for i in page.clone() {
            let candidate = &self.snapshot.candidates[i];
            let measured = &self.layout.items[i];
            let position = self.layout.positions[i - page.start];
            let is_selected = selected == i;
            let candidate_id = candidate.id;
            let mut item = div()
                .id(("candidate", i))
                .absolute()
                .left(position.x)
                .top(position.y)
                .w(if self.layout.vertical {
                    self.layout.content_size.width
                } else {
                    px(measured.width)
                })
                .h(px(measured.height).min(self.layout.content_size.height - position.y))
                .flex_shrink_0()
                .overflow_y_scroll()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(INLINE_GAP))
                .px(px(skin.candidate.padding_x))
                .py(px(skin.candidate.padding_y))
                .rounded(px(skin.candidate.radius))
                .bg(if is_selected {
                    selection_background
                } else {
                    Background::from(rgba(0x00000000))
                })
                .text_color(rgba(if is_selected {
                    skin.candidate.selected_text.0
                } else {
                    skin.candidate.text.0
                }))
                .text_size(px(skin.candidate.font_size))
                .line_height(px(skin.candidate.font_size * 1.5))
                .when(!is_selected, |item| {
                    item.hover(|style| style.bg(rgba(skin.candidate.hover_background.0)))
                })
                .on_click(cx.listener(move |panel, _, window, cx| {
                    if let Some(id) = context_id {
                        panel.confirm_selection(id, candidate_id, window, cx);
                    } else {
                        panel.preview_selection(i, window, cx);
                    }
                }));
            if is_selected {
                item = item.opacity(selection_motion.opacity);
                if let Some(progress) = selection_motion.sweep {
                    item = item.child(effects::shimmer(progress, skin.candidate.radius));
                }
                let mut shadows = Vec::new();
                if let Some(color) = skin.candidate.selected_glow {
                    shadows.push(
                        BoxShadow::new(px(0.0), px(0.0), rgba(color.0).into())
                            .blur_radius(px(10.0)),
                    );
                    shadows.push(
                        BoxShadow::new(px(0.0), px(0.0), rgba(color.0).into())
                            .blur_radius(px(8.0))
                            .inset(),
                    );
                }
                if let Some(color) = skin.candidate.selected_border {
                    shadows.push(
                        BoxShadow::new(px(0.0), px(0.0), rgba(color.0).into())
                            .spread_radius(px(1.0))
                            .inset(),
                    );
                }
                item = item.shadow(shadows);
            }
            if skin.layout.show_labels {
                item = item.child(
                    div()
                        .w(px(self.layout.label_width))
                        .flex_shrink_0()
                        .font_weight(FontWeight::MEDIUM)
                        .text_size(px((skin.candidate.font_size - 2.0).max(10.0)))
                        .text_color(rgba(if is_selected {
                            skin.candidate.selected_label.0
                        } else {
                            skin.candidate.label.0
                        }))
                        .child((i - page.start + 1).to_string()),
                );
            }
            let mut content = div().flex().w(px(measured.text_width)).flex_shrink_0();
            content = if measured.stacked {
                content.flex_col().gap(px(2.0))
            } else {
                content.flex_row().gap(px(INLINE_GAP)).items_center()
            };
            let mut text = div().flex_shrink_0().child(candidate.text.clone());
            text = if measured.stacked {
                text.w_full().whitespace_normal()
            } else {
                text.whitespace_nowrap()
            };
            content = content.child(text);
            if let Some(comment) = &measured.annotation {
                let font_size = (skin.candidate.font_size - 2.0).max(10.0);
                let mut annotation = div()
                    .flex_shrink_0()
                    .text_size(px(font_size))
                    .line_height(px(font_size * 1.5))
                    .text_color(rgba(if is_selected {
                        skin.candidate.selected_label.0
                    } else {
                        skin.candidate.comment.0
                    }))
                    .child(comment.clone());
                annotation = if measured.stacked {
                    annotation.w_full().whitespace_normal()
                } else {
                    annotation.whitespace_nowrap()
                };
                content = content.child(annotation);
            }
            list = list.child(item.child(content));
        }
        let background = skin
            .window
            .background_alt
            .map(|alternate| {
                linear_gradient(
                    skin.window.gradient_angle,
                    linear_color_stop(rgba(skin.window.background.0), 0.0),
                    linear_color_stop(rgba(alternate.0), 1.0),
                )
            })
            .unwrap_or_else(|| Background::from(rgba(skin.window.background.0)));
        root = root
            .flex()
            .flex_col()
            .pt(px(self.layout.top_padding))
            .pr(px(skin.canvas.content_right + skin.layout.padding))
            .pb(px(skin.canvas.content_bottom + skin.layout.padding))
            .pl(px(skin.canvas.content_left + skin.layout.padding))
            .bg(background)
            .backdrop_blur(px(skin.window.backdrop_blur))
            .border(px(skin.window.border_width))
            .border_color(rgba(0x00000000))
            .rounded(px(skin.window.radius));
        if skin.window.shadow {
            root = root.shadow_lg();
        }
        if self.layout.header_height > 0.0 {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .self_start()
                    .w(px(self.layout.preedit_width.ceil()))
                    .max_w_full()
                    .h(px(self.layout.header_height - skin.layout.gap))
                    .mb(px(skin.layout.gap))
                    .px(px(skin.preedit.padding_x))
                    .py(px(skin.preedit.padding_y))
                    .bg(rgba(skin.preedit.background.0))
                    .rounded(px(skin.preedit.radius))
                    .overflow_hidden()
                    .text_size(px(skin.preedit.font_size))
                    .line_height(px(skin.preedit.font_size * 1.5))
                    .text_color(rgba(skin.preedit.color.0))
                    .border_b_1()
                    .border_color(rgba(skin.preedit.separator.0))
                    .child(
                        div().min_w_0().truncate().child(
                            gpui::StyledText::new(self.snapshot.panel_text().to_owned())
                                .with_highlights(
                                    mirix_core::preedit_selection(
                                        self.snapshot.panel_text(),
                                        self.snapshot.panel_selection(),
                                    )
                                    .map(|range| {
                                        (
                                            range,
                                            gpui::HighlightStyle {
                                                color: Some(
                                                    rgba(skin.candidate.selected_text.0).into(),
                                                ),
                                                background_color: Some(
                                                    rgba(skin.candidate.selected_background.0)
                                                        .into(),
                                                ),
                                                ..Default::default()
                                            },
                                        )
                                    }),
                                ),
                        ),
                    ),
            );
        }
        let inline = self.layout.inline_navigation;
        let mut body = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(skin.layout.gap))
            .child(list);
        let mut buttons = div().flex().flex_shrink_0().gap(px(4.0)).h(px(NAV_HEIGHT));
        if self.layout.navigation {
            buttons = if inline {
                buttons.w(px(NAV_WIDTH))
            } else {
                buttons.w_full().mt(px(8.0))
            };
            for (forward, enabled) in [
                (false, page.start > 0),
                (true, page.end < self.snapshot.candidates.len()),
            ] {
                let queue = commands.clone();
                buttons = buttons.child(
                    div()
                        .id(if forward {
                            "next-page"
                        } else {
                            "previous-page"
                        })
                        .flex_1()
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(skin.navigation.radius))
                        .bg(rgba(skin.navigation.background.0))
                        .border_1()
                        .border_color(rgba(skin.navigation.border.0))
                        .opacity(if enabled { 1.0 } else { 0.4 })
                        .when(enabled, |button| {
                            button
                                .cursor_pointer()
                                .hover(|style| style.bg(rgba(skin.candidate.hover_background.0)))
                        })
                        .on_click(cx.listener(move |panel, _, window, cx| {
                            if !enabled || panel.quick_switch.is_some() {
                                return;
                            }
                            if let Some(id) = context_id {
                                navigate_candidates(
                                    queue.clone(),
                                    id,
                                    if forward {
                                        keysym::PAGE_DOWN
                                    } else {
                                        keysym::PAGE_UP
                                    },
                                );
                            } else {
                                let target = if forward {
                                    panel.layout.page.end
                                } else {
                                    panel.layout.page.start.saturating_sub(1)
                                };
                                panel.preview_selection(target, window, cx);
                            }
                        }))
                        .child(chevron(forward, !inline, skin.navigation.foreground.0)),
                );
            }
        }
        if inline && self.layout.navigation {
            body = body.child(buttons);
            root = root.child(body);
        } else {
            root = root.child(body);
            if self.layout.navigation {
                root = root.child(buttons);
            }
        }
        if let Some(scene) = &scene {
            root = root.children(scene.render(LayerPlane::Foreground));
        }
        // A quiet corner annotation, outside the flow and above skin artwork.
        // Its presence or value never changes candidate widths or pagination.
        if let Some(mode) = self.snapshot.punctuation_mode {
            root = root.child(
                div()
                    .absolute()
                    .right(px((skin.window.radius * 0.75).max(5.0)))
                    .bottom(px(2.0))
                    .size(px(6.0))
                    .rounded_full()
                    .overflow_hidden()
                    .border(px(0.7))
                    .border_color(rgba(skin.candidate.comment.0))
                    .opacity(0.7)
                    .when(mode == PunctuationMode::Full, |marker| {
                        marker.bg(rgba(skin.candidate.comment.0))
                    })
                    .when(mode == PunctuationMode::Half, |marker| {
                        marker.child(
                            div()
                                .absolute()
                                .left(px(0.0))
                                .top(px(0.0))
                                .bottom(px(0.0))
                                .w(px(2.3))
                                .bg(rgba(skin.candidate.comment.0)),
                        )
                    }),
            );
        }
        let body_opacity = if animations_enabled {
            self.motion.values(skin, "panel", now).opacity
        } else {
            1.0
        };
        root = root.opacity(body_opacity);
        // Overflow masks are rectangular in GPUI; clip the whole subtree so
        // backgrounds, decorations and particles share the body's rounded edge.
        surface = surface.child(effects::rounded_clip(root, skin.window.radius));
        // The continuous closed rim is always above art, particles and the body.
        surface = surface.child(panel_frame(&skin.window, body_bounds).opacity(body_opacity));
        if self.quick_switch.is_some() {
            surface = surface.child(self.render_quick_switch(cx));
        }
        // Visibility owns the clock, not an element ID retained across unmaps.
        if matches!(skin.transitions.show, MotionStyle::Fade) {
            surface = surface.opacity(show_progress);
        }
        surface.into_any_element()
    }
}

/// Geometric arrows keep the same stroke and alignment across font families.
fn chevron(forward: bool, vertical: bool, color: u32) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let sign = if forward { 1.0 } else { -1.0 };
            let points = [(-2.5 * sign, -5.0), (2.5 * sign, 0.0), (-2.5 * sign, 5.0)];
            let center = bounds.center();
            let mut path = PathBuilder::stroke(px(1.7));
            for (i, (x, y)) in points.into_iter().enumerate() {
                let (x, y) = if vertical { (y, x) } else { (x, y) };
                let p = center + point(px(x), px(y));
                if i == 0 {
                    path.move_to(p);
                } else {
                    path.line_to(p);
                }
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, rgba(color));
            }
        },
    )
    .w(px(18.0))
    .h(px(18.0))
}

/// The same outer bounds/radius as the body: no independently joined border segments.
fn panel_frame(style: &WindowStyle, bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Div {
    let mut frame = div()
        .absolute()
        .left(bounds.origin.x)
        .top(bounds.origin.y)
        .w(bounds.size.width)
        .h(bounds.size.height)
        .rounded(px(style.radius))
        .border(px(style.border_width))
        .border_color(rgba(style.border.0));
    if let Some(highlight) = style.border_highlight {
        frame = frame.border_gradient(border_gradient([
            border_color_stop(rgba(highlight.0), 0.0),
            border_color_stop(rgba(style.border.0), 0.35),
            border_color_stop(rgba(style.border.0), 0.75),
            border_color_stop(rgba(highlight.0), 1.0),
        ]));
    }
    if let Some(glow) = style.glow {
        frame = frame.shadow(vec![
            BoxShadow::new(px(0.0), px(0.0), rgba(glow.0).into())
                .blur_radius(px(10.0))
                .spread_radius(px(2.0))
                .inset(),
        ]);
    }
    frame
}
