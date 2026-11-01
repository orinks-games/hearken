use super::draw::{Layer, Painter, Rgba};
use super::text::{Atlas, Text, Weight, ATLAS_SIZE};
use super::{Scene, Shape};
use sdl2::render::{BlendMode, Canvas, RenderTarget, Texture};

const INK: Rgba = Rgba::rgb(15, 22, 30);
const PANEL: Rgba = Rgba::rgb(26, 38, 49);
const WHITE: Rgba = Rgba::rgb(244, 240, 224);
const GOLD: Rgba = Rgba::rgb(255, 206, 112);

/// Owns the CPU glyph atlas and draw batches. The shell owns its GPU texture.
pub struct Renderer {
    pub text: Text,
    pub painter: Painter,
    last: Option<(Scene, (u32, u32))>,
    row_scroll: i32,
    caption_scroll: usize,
}

impl Default for Renderer {
    fn default() -> Self {
        Self {
            text: Text::new(),
            painter: Painter::new(Atlas::white_uv()),
            last: None,
            row_scroll: 0,
            caption_scroll: 0,
        }
    }
}

impl Renderer {
    /// Positive scroll moves down through the screen or forward through captions.
    pub fn scroll(&mut self, delta: i32, captions: bool) {
        if captions {
            self.caption_scroll = self.caption_scroll.saturating_add_signed(-delta as isize);
        } else {
            self.row_scroll = self.row_scroll.saturating_add(delta);
        }
    }

    /// False means that the existing window pixels already show this frame.
    pub fn build(&mut self, scene: &Scene, size: (u32, u32), force: bool) -> bool {
        if !force
            && self
                .last
                .as_ref()
                .is_some_and(|(s, wh)| s == scene && *wh == size)
        {
            return false;
        }
        if self.last.as_ref().is_some_and(|(old, _)| {
            old.lines.iter().find(|line| line.starts_with("> "))
                != scene.lines.iter().find(|line| line.starts_with("> "))
        }) {
            self.row_scroll = 0;
        }
        self.last = Some((scene.clone(), size));
        self.painter.clear();
        self.text.begin_frame();
        let ink = if scene.high_contrast {
            Rgba::BLACK
        } else {
            INK
        };
        let panel = if scene.high_contrast {
            Rgba::BLACK
        } else {
            PANEL
        };
        let white = if scene.high_contrast {
            Rgba::WHITE
        } else {
            WHITE
        };
        let gold = if scene.high_contrast {
            Rgba::rgb(255, 255, 0)
        } else {
            GOLD
        };
        let (w, h) = (size.0 as f32, size.1 as f32);
        self.painter.rect(0., 0., w, h, ink);
        if !scene.enabled {
            return true;
        }
        let scale = (h / 720.).clamp(0.75, 2.5);
        let text_scale = scale * f32::from(scene.text_percent.clamp(100, 200)) / 100.;
        let margin = 24. * scale;
        let caption_h = 110. * text_scale;
        let body_h = (h - caption_h - margin * 2.).max(1.);
        let has_art = !scene.shapes.is_empty() && w >= 800. * scale;
        let panel_x = if has_art { w * 0.62 } else { margin };
        if has_art {
            let art_w = panel_x - margin * 2.;
            let zoom = (art_w / 1000.).min(body_h / 600.);
            let ox = margin + (art_w - 1000. * zoom) / 2.;
            let oy = margin + (body_h - 600. * zoom) / 2.;
            for shape in &scene.shapes {
                let color = |c: &[u8; 3]| Rgba::rgb(c[0], c[1], c[2]);
                match shape {
                    Shape::Rect {
                        x,
                        y,
                        w,
                        h,
                        color: c,
                    } => self.painter.rect(
                        ox + x * zoom,
                        oy + y * zoom,
                        w.max(0.) * zoom,
                        h.max(0.) * zoom,
                        color(c),
                    ),
                    Shape::Circle { x, y, r, color: c } => self.painter.circle(
                        ox + x * zoom,
                        oy + y * zoom,
                        r.max(0.) * zoom,
                        color(c),
                    ),
                    Shape::Line {
                        from,
                        to,
                        width,
                        color: c,
                    } => self.painter.line(
                        (ox + from.0 * zoom, oy + from.1 * zoom),
                        (ox + to.0 * zoom, oy + to.1 * zoom),
                        width.max(1.) * zoom,
                        color(c),
                    ),
                    Shape::Label {
                        x,
                        y,
                        text,
                        color: c,
                    } => {
                        self.text.draw(
                            &mut self.painter,
                            ox + x * zoom,
                            oy + y * zoom,
                            text,
                            30. * zoom,
                            Weight::Bold,
                            color(c),
                        );
                    }
                }
            }
        }
        self.painter.set_layer(Layer::Hud);
        let panel_w = (w - panel_x - margin).max(1.);
        self.painter
            .round_rect(panel_x, margin, panel_w, body_h, 12. * scale, panel);
        let x = panel_x + 18. * scale;
        let width = (panel_w - 36. * scale).max(1.);
        let size = 26. * text_scale;
        let line_h = 32. * text_scale;
        let mut y = margin + 38. * text_scale;
        // Wrap before choosing the visible range, so the focused row stays
        // visible even when preceding labels take several physical lines.
        let mut rows = Vec::new();
        for (i, line) in scene.lines.iter().enumerate() {
            let focused = line.starts_with("> ");
            let value = line
                .strip_prefix("> ")
                .or_else(|| line.strip_prefix("  "))
                .unwrap_or(line);
            let weight = if i == 0 || focused {
                Weight::Bold
            } else {
                Weight::Regular
            };
            for part in self.text.wrap(value, size, weight, width - 18. * scale) {
                rows.push((part, focused, weight));
            }
        }
        let capacity = ((body_h - 48. * text_scale) / line_h).max(1.) as usize;
        let focus = rows.iter().position(|r| r.1).unwrap_or(0);
        let start = focus
            .saturating_sub(capacity / 2)
            .min(rows.len().saturating_sub(capacity));
        let base = start;
        let start = base
            .saturating_add_signed(self.row_scroll as isize)
            .min(rows.len().saturating_sub(capacity));
        self.row_scroll = start as i32 - base as i32;
        for (line, focused, weight) in rows.iter().skip(start).take(capacity) {
            if *focused {
                self.painter.round_rect(
                    x - 6. * scale,
                    y - 26. * text_scale,
                    width + 8. * scale,
                    line_h,
                    4. * scale,
                    gold,
                );
                self.painter.rect(
                    x - 12. * scale,
                    y - 26. * text_scale,
                    4. * scale,
                    line_h,
                    white,
                );
            }
            self.text.draw(
                &mut self.painter,
                x + 4. * scale,
                y,
                line,
                size,
                *weight,
                if *focused { ink } else { white },
            );
            y += line_h;
        }
        if start > 0 {
            self.painter.line(
                (x, margin + 5. * scale),
                (x + width, margin + 5. * scale),
                2. * scale,
                gold,
            );
        }
        if start + capacity < rows.len() {
            self.painter.line(
                (x, margin + body_h - 5. * scale),
                (x + width, margin + body_h - 5. * scale),
                2. * scale,
                gold,
            );
        }
        let cap_y = h - caption_h + 10. * scale;
        self.painter.rect(
            margin,
            cap_y - 20. * scale,
            w - 2. * margin,
            2. * scale,
            gold,
        );
        let mut captions = Vec::new();
        let source = if scene.captions.is_empty() {
            std::slice::from_ref(&scene.caption)
        } else {
            &scene.captions
        };
        for line in source {
            captions.extend(self.text.wrap(line, size, Weight::Regular, w - 2. * margin));
        }
        self.caption_scroll = self.caption_scroll.min(captions.len().saturating_sub(3));
        let start = captions.len().saturating_sub(3 + self.caption_scroll);
        for (i, line) in captions.iter().skip(start).take(3).enumerate() {
            self.text.draw(
                &mut self.painter,
                margin,
                cap_y - 20. * scale + 30. * text_scale + i as f32 * line_h,
                line,
                size,
                Weight::Regular,
                white,
            );
        }
        true
    }

    pub fn draw<T: RenderTarget>(
        &mut self,
        canvas: &mut Canvas<T>,
        atlas: &mut Texture,
    ) -> Result<(), String> {
        if let Some([x, y, w, h]) = self.text.atlas.take_dirty() {
            let pitch = ATLAS_SIZE as usize * 4;
            let start = y as usize * pitch + x as usize * 4;
            atlas
                .update(
                    sdl2::rect::Rect::new(x as i32, y as i32, w, h),
                    &self.text.atlas.pixels[start..],
                    pitch,
                )
                .map_err(|e| e.to_string())?;
        }
        canvas.set_draw_color(INK.to_color());
        canvas.clear();
        for (i, batch) in self.painter.batches.iter().enumerate() {
            if batch.indices.is_empty() {
                continue;
            }
            atlas.set_blend_mode(if i == Layer::Glow as usize {
                BlendMode::Add
            } else {
                BlendMode::Blend
            });
            canvas.render_geometry(&batch.vertices, Some(atlas), batch.indices.as_slice())?;
        }
        Ok(())
    }
}

/// Render a snapshot without opening a window, for visual regression review.
pub fn save_bmp(scene: &Scene, size: (u32, u32), path: &std::path::Path) -> Result<(), String> {
    use sdl2::{pixels::PixelFormatEnum, surface::Surface};
    let surface = Surface::new(size.0, size.1, PixelFormatEnum::RGBA32)?;
    let mut canvas = surface.into_canvas().map_err(|e| e.to_string())?;
    let mut atlas = canvas
        .texture_creator()
        .create_texture_streaming(PixelFormatEnum::RGBA32, ATLAS_SIZE, ATLAS_SIZE)
        .map_err(|e| e.to_string())?;
    let mut renderer = Renderer::default();
    renderer.build(scene, size, true);
    renderer.draw(&mut canvas, &mut atlas)?;
    canvas.surface().save_bmp(path)
}
