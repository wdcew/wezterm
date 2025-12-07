use crate::quad::{
    QuadImpl, QuadTrait, TripleLayerQuadAllocator, TripleLayerQuadAllocatorTrait, V_BOT_LEFT,
    V_BOT_RIGHT, V_TOP_LEFT, V_TOP_RIGHT,
};
use config::HsbTransform;
use mux::renderable::StableCursorPosition;
use std::ops::Range;
use std::time::Instant;
use wezterm_term::StableRowIndex;
use window::bitmaps::TextureRect;
use window::color::LinearRgba;

/// Distance threshold for considering corners "at cursor"
const SETTLED_THRESHOLD: f32 = 0.1;

/// Manages the cursor trail effect with a deformable quad
#[derive(Debug)]
pub struct CursorTrail {
    /// Four corners of the trail quad: top-left, top-right, bottom-right, bottom-left
    corners: [(f32, f32); 4],

    /// Trail target bounds (where corners are animating towards)
    target_left: f32,
    target_right: f32,
    target_top: f32,
    target_bottom: f32,

    /// Current cursor position
    cursor_pos: Option<StableCursorPosition>,

    /// When the cursor last moved to a new position
    cursor_last_moved: Instant,

    /// Timestamp of last update
    updated_at: Instant,
}

impl CursorTrail {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            corners: [(0.0, 0.0); 4],
            target_left: 0.0,
            target_right: 0.0,
            target_top: 0.0,
            target_bottom: 0.0,
            cursor_pos: None,
            cursor_last_moved: now,
            updated_at: now,
        }
    }

    /// Update the trail state and return true if the quad should be rendered.
    pub fn update(
        &mut self,
        cursor_pos: &StableCursorPosition,
        min_distance: f32,
        decay_fast: f32,
        decay_slow: f32,
        dwell_time_ms: u64,
    ) -> bool {
        let now = Instant::now();
        let delta_time = now.duration_since(self.updated_at).as_secs_f32();
        self.updated_at = now;

        let cursor_x = cursor_pos.x as f32;
        let cursor_y = cursor_pos.y as f32;

        if self
            .cursor_pos
            .as_ref()
            .map_or(true, |last_pos| last_pos != cursor_pos)
        {
            self.cursor_last_moved = now;
            self.cursor_pos = Some(*cursor_pos);
        }

        let dwell_time = now.duration_since(self.cursor_last_moved).as_millis() as u64;
        let target_to_cursor_distance =
            (cursor_x - self.target_left).abs() + (cursor_y - self.target_top).abs();

        if dwell_time >= dwell_time_ms && target_to_cursor_distance > min_distance {
            self.target_left = cursor_x;
            self.target_right = cursor_x + 1.0;
            self.target_top = cursor_y;
            self.target_bottom = cursor_y + 1.0;
        }

        if self.target_left == 0.0 && self.target_right == 0.0 {
            self.target_left = cursor_x;
            self.target_right = cursor_x + 1.0;
            self.target_top = cursor_y;
            self.target_bottom = cursor_y + 1.0;
            self.corners = [
                (cursor_x, cursor_y),
                (cursor_x + 1.0, cursor_y),
                (cursor_x + 1.0, cursor_y + 1.0),
                (cursor_x, cursor_y + 1.0),
            ];
            return false;
        }

        if target_to_cursor_distance > 0.0 && target_to_cursor_distance <= min_distance {
            self.corners = [
                (cursor_x, cursor_y),
                (cursor_x + 1.0, cursor_y),
                (cursor_x + 1.0, cursor_y + 1.0),
                (cursor_x, cursor_y + 1.0),
            ];
            self.target_left = cursor_x;
            self.target_right = cursor_x + 1.0;
            self.target_top = cursor_y;
            self.target_bottom = cursor_y + 1.0;
            return false;
        }

        // Animate corners towards target using exponential ease-out
        let target_x = [
            self.target_left,
            self.target_right,
            self.target_right,
            self.target_left,
        ];
        let target_y = [
            self.target_top,
            self.target_top,
            self.target_bottom,
            self.target_bottom,
        ];

        let target_center_x = (self.target_left + self.target_right) * 0.5;
        let target_center_y = (self.target_top + self.target_bottom) * 0.5;
        let target_width = self.target_right - self.target_left;
        let target_height = self.target_bottom - self.target_top;
        let target_diag_2 = (target_width.powi(2) + target_height.powi(2)).sqrt() * 0.5;

        let mut dx = [0.0_f32; 4];
        let mut dy = [0.0_f32; 4];
        let mut dot = [0.0_f32; 4];

        for i in 0..4 {
            dx[i] = target_x[i] - self.corners[i].0;
            dy[i] = target_y[i] - self.corners[i].1;

            if dx[i].abs() < 1e-6 && dy[i].abs() < 1e-6 {
                dx[i] = 0.0;
                dy[i] = 0.0;
                dot[i] = 0.0;
            } else {
                let norm = (dx[i].powi(2) + dy[i].powi(2)).sqrt();
                let corner_to_center_x = target_x[i] - target_center_x;
                let corner_to_center_y = target_y[i] - target_center_y;
                dot[i] = (dx[i] * corner_to_center_x + dy[i] * corner_to_center_y)
                    / (target_diag_2 * norm);
            }
        }

        let min_dot = dot.iter().cloned().fold(f32::INFINITY, f32::min);
        let max_dot = dot.iter().cloned().fold(f32::NEG_INFINITY, f32::max);

        for i in 0..4 {
            if (dx[i] == 0.0 && dy[i] == 0.0) || min_dot.is_infinite() {
                continue;
            }

            let decay = if (max_dot - min_dot).abs() < 1e-6 {
                decay_slow
            } else {
                decay_slow + (decay_fast - decay_slow) * (dot[i] - min_dot) / (max_dot - min_dot)
            };

            let step = 1.0 - 2.0_f32.powf(-10.0 * delta_time / decay);
            self.corners[i].0 += dx[i] * step;
            self.corners[i].1 += dy[i] * step;
        }

        let waiting_for_dwell =
            target_to_cursor_distance > min_distance && dwell_time < dwell_time_ms;
        !self.settled(SETTLED_THRESHOLD) || waiting_for_dwell
    }

    fn settled(&self, threshold: f32) -> bool {
        for i in 0..4 {
            let target_x = if i == 1 || i == 2 {
                self.target_right
            } else {
                self.target_left
            };
            let target_y = if i >= 2 {
                self.target_bottom
            } else {
                self.target_top
            };
            let dx = target_x - self.corners[i].0;
            let dy = target_y - self.corners[i].1;
            if dx.abs() > threshold || dy.abs() > threshold {
                return false;
            }
        }
        true
    }

    pub fn render(
        &self,
        layers: &mut TripleLayerQuadAllocator,
        cell_width: f32,
        cell_height: f32,
        pane_left: usize,
        stable_range: Range<StableRowIndex>,
        window_dimensions: (f32, f32), // (width, height)
        pixel_offset: (f32, f32),      // (left_pixel_x, top_pixel_y)
        trail_color: LinearRgba,
        hsv_transform: Option<HsbTransform>,
        white_space_texture: TextureRect,
    ) -> anyhow::Result<()> {
        let (window_width, window_height) = window_dimensions;
        let (left_pixel_x, top_pixel_y) = pixel_offset;

        // Convert corner positions from cell coordinates to pixel coordinates
        let px_x = (window_width / -2.0) + left_pixel_x;
        let px_y = (window_height / -2.0) + top_pixel_y;

        let pixel_corners = [
            [
                px_x + (self.corners[0].0 - pane_left as f32) * cell_width,
                px_y + (self.corners[0].1 - stable_range.start as f32) * cell_height,
            ],
            [
                px_x + (self.corners[1].0 - pane_left as f32) * cell_width,
                px_y + (self.corners[1].1 - stable_range.start as f32) * cell_height,
            ],
            [
                px_x + (self.corners[3].0 - pane_left as f32) * cell_width,
                px_y + (self.corners[3].1 - stable_range.start as f32) * cell_height,
            ],
            [
                px_x + (self.corners[2].0 - pane_left as f32) * cell_width,
                px_y + (self.corners[2].1 - stable_range.start as f32) * cell_height,
            ],
        ];

        let mut quad_impl = layers.allocate(0)?;

        match &mut quad_impl {
            QuadImpl::Vert(quad) => {
                quad.vert[V_TOP_LEFT].position = pixel_corners[0];
                quad.vert[V_TOP_RIGHT].position = pixel_corners[1];
                quad.vert[V_BOT_LEFT].position = pixel_corners[2];
                quad.vert[V_BOT_RIGHT].position = pixel_corners[3];
            }
            QuadImpl::Boxed(_) => {}
        }

        quad_impl.set_hsv(hsv_transform);
        quad_impl.set_is_background();
        quad_impl.set_texture(white_space_texture);
        quad_impl.set_fg_color(trail_color);

        Ok(())
    }
}
