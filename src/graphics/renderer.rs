//! Image buffer downscaling, aspect-ratio preservation, and protocol rendering adapter.

use image::imageops::FilterType;
use image::DynamicImage;
use ratatui::layout::{Rect, Size};
use ratatui_image::picker::Picker;
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

use crate::error::AppError;

/// Preserved aspect-ratio fit bounds within a terminal pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FitBounds {
    /// Target terminal character cell area, centered within the container pane.
    pub render_area: Rect,
    /// Target pixel dimensions for graphical protocols (Kitty/Sixel).
    pub target_pixel_size: (u32, u32),
}

pub struct ImageRenderer;

impl ImageRenderer {
    /// Calculates aspect-ratio preserved fit bounds inside a target pane Rect,
    /// correctly accounting for non-square terminal font cells (typically 1:2 ratio).
    pub fn calculate_fit(
        img_width: u32,
        img_height: u32,
        font_size: ratatui_image::FontSize,
        pane: Rect,
    ) -> FitBounds {
        if img_width == 0
            || img_height == 0
            || pane.width == 0
            || pane.height == 0
            || font_size.width == 0
            || font_size.height == 0
        {
            return FitBounds {
                render_area: Rect::default(),
                target_pixel_size: (0, 0),
            };
        }

        let cell_w = font_size.width as f32;
        let cell_h = font_size.height as f32;

        // Effective ratio of the image expressed in character cell units:
        // (W_img / H_img) * (H_cell / W_cell)
        let r_cells = (img_width as f32 / img_height as f32) * (cell_h / cell_w);
        let r_pane = pane.width as f32 / pane.height as f32;

        let (fit_w, fit_h) = if r_cells >= r_pane {
            // Width-constrained: fills pane width, scales height
            let w = pane.width;
            let raw_h = (w as f32 / r_cells).round() as u16;
            let h = raw_h.clamp(1, pane.height);
            (w, h)
        } else {
            // Height-constrained: fills pane height, scales width
            let h = pane.height;
            let raw_w = (h as f32 * r_cells).round() as u16;
            let w = raw_w.clamp(1, pane.width);
            (w, h)
        };

        // Center within container pane
        let offset_x = pane.x + (pane.width.saturating_sub(fit_w)) / 2;
        let offset_y = pane.y + (pane.height.saturating_sub(fit_h)) / 2;

        let render_area = Rect {
            x: offset_x,
            y: offset_y,
            width: fit_w,
            height: fit_h,
        };

        let target_pixel_w = fit_w as u32 * font_size.width as u32;
        let target_pixel_h = fit_h as u32 * font_size.height as u32;

        FitBounds {
            render_area,
            target_pixel_size: (target_pixel_w, target_pixel_h),
        }
    }

    /// Pre-downscales the image in the background and encodes it into a ready-to-render Protocol.
    /// Prevents blocking the 60 FPS UI thread during large image resampling.
    pub fn prepare_protocol(
        picker: &Picker,
        img: &DynamicImage,
        pane: Rect,
        filter: FilterType,
    ) -> Result<(Protocol, Rect), AppError> {
        let fit = Self::calculate_fit(img.width(), img.height(), picker.font_size(), pane);
        if fit.render_area.width == 0 || fit.render_area.height == 0 {
            return Err(AppError::Image(
                "Pane area or image dimension is zero".to_string(),
            ));
        }

        // Downscale before protocol encoding to conserve memory and avoid UI thread latency
        let downscaled =
            if picker.protocol_type() == ratatui_image::picker::ProtocolType::Halfblocks {
                // Halfblocks requires width cells and height * 2 pixels
                img.resize_exact(
                    fit.render_area.width as u32,
                    (fit.render_area.height as u32 * 2).max(1),
                    filter,
                )
            } else {
                img.resize_exact(
                    fit.target_pixel_size.0.max(1),
                    fit.target_pixel_size.1.max(1),
                    filter,
                )
            };

        let size = Size::new(fit.render_area.width, fit.render_area.height);
        let proto = picker
            .new_protocol(downscaled, size, Resize::Fit(Some(filter)))
            .map_err(|e| AppError::Image(format!("Protocol encode error: {e}")))?;

        Ok((proto, fit.render_area))
    }
}
