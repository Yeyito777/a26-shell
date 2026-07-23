use std::error::Error;

use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeGCAux, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask,
    Gcontext, Pixmap, Rectangle, StackMode, Window, WindowClass,
};

use crate::font;
use crate::ui::{ACCENT, BG_CARD, FG, MUTED};

pub const WIDTH: u16 = 240;
pub const HEIGHT: u16 = 720;
pub const X: i16 = 790;
pub const Y: i16 = 330;

pub struct VolumeSurface {
    pub window: Window,
    pixmap: Pixmap,
    gc: Gcontext,
    mapped: bool,
    redraw: bool,
    last_volume: Option<u8>,
}

impl VolumeSurface {
    pub fn create<C: Connection>(
        conn: &C,
        root: Window,
        depth: u8,
    ) -> Result<Self, Box<dyn Error>> {
        let window = conn.generate_id()?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            X,
            Y,
            WIDTH,
            HEIGHT,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(BG_CARD)
                .override_redirect(1)
                .event_mask(EventMask::EXPOSURE),
        )?;
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, pixmap, window, WIDTH, HEIGHT)?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &CreateGCAux::new().graphics_exposures(0))?;
        Ok(Self {
            window,
            pixmap,
            gc,
            mapped: false,
            redraw: true,
            last_volume: None,
        })
    }

    pub fn request_redraw(&mut self) {
        self.redraw = true;
    }

    pub fn sync<C: Connection>(
        &mut self,
        conn: &C,
        visible: bool,
        volume: u8,
    ) -> Result<(), Box<dyn Error>> {
        if !visible {
            if self.mapped {
                conn.unmap_window(self.window)?;
                self.mapped = false;
            }
            return Ok(());
        }

        let newly_mapped = !self.mapped;
        if newly_mapped {
            conn.map_window(self.window)?;
            self.mapped = true;
            self.redraw = true;
        }
        let needs_present = self.redraw || self.last_volume != Some(volume);
        if newly_mapped || needs_present {
            conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
        }
        if needs_present {
            self.render(conn, volume)?;
            self.redraw = false;
            self.last_volume = Some(volume);
        }
        Ok(())
    }

    fn render<C: Connection>(&self, conn: &C, volume: u8) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 0,
                y: 0,
                width: WIDTH,
                height: HEIGHT,
            },
        )?;
        self.center_text(conn, "VOL", 45, 5, FG)?;
        self.outline(
            conn,
            MUTED,
            Rectangle {
                x: 85,
                y: 145,
                width: 70,
                height: 470,
            },
            4,
        )?;
        let filled = u16::from(volume) * 454 / 100;
        self.fill(
            conn,
            ACCENT,
            Rectangle {
                x: 93,
                y: (607 - filled) as i16,
                width: 54,
                height: filled,
            },
        )?;
        self.center_text(conn, &volume.to_string(), 640, 5, ACCENT)?;
        conn.copy_area(self.pixmap, self.window, self.gc, 0, 0, 0, 0, WIDTH, HEIGHT)?;
        Ok(())
    }

    fn center_text<C: Connection>(
        &self,
        conn: &C,
        text: &str,
        y: i16,
        scale: u16,
        color: u32,
    ) -> Result<(), Box<dyn Error>> {
        let width = font::text_width(text, scale);
        let x = ((WIDTH.saturating_sub(width)) / 2) as i16;
        conn.change_gc(self.gc, &ChangeGCAux::new().foreground(color))?;
        let rectangles = font::rectangles(text, x, y, scale);
        if !rectangles.is_empty() {
            conn.poly_fill_rectangle(self.pixmap, self.gc, &rectangles)?;
        }
        Ok(())
    }

    fn fill<C: Connection>(
        &self,
        conn: &C,
        color: u32,
        rectangle: Rectangle,
    ) -> Result<(), Box<dyn Error>> {
        conn.change_gc(self.gc, &ChangeGCAux::new().foreground(color))?;
        conn.poly_fill_rectangle(self.pixmap, self.gc, &[rectangle])?;
        Ok(())
    }

    fn outline<C: Connection>(
        &self,
        conn: &C,
        color: u32,
        rectangle: Rectangle,
        thickness: u16,
    ) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            color,
            Rectangle {
                x: rectangle.x,
                y: rectangle.y,
                width: rectangle.width,
                height: thickness,
            },
        )?;
        self.fill(
            conn,
            color,
            Rectangle {
                x: rectangle.x,
                y: rectangle.y + rectangle.height as i16 - thickness as i16,
                width: rectangle.width,
                height: thickness,
            },
        )?;
        self.fill(
            conn,
            color,
            Rectangle {
                x: rectangle.x,
                y: rectangle.y,
                width: thickness,
                height: rectangle.height,
            },
        )?;
        self.fill(
            conn,
            color,
            Rectangle {
                x: rectangle.x + rectangle.width as i16 - thickness as i16,
                y: rectangle.y,
                width: thickness,
                height: rectangle.height,
            },
        )?;
        Ok(())
    }

    pub fn destroy<C: Connection>(&self, conn: &C) {
        let _ = conn.destroy_window(self.window);
        let _ = conn.free_pixmap(self.pixmap);
        let _ = conn.free_gc(self.gc);
    }
}
