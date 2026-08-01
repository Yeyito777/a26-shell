use std::error::Error;

use x11rb::COPY_DEPTH_FROM_PARENT;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeGCAux, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux,
    EventMask, Gcontext, Pixmap, PropMode, Rectangle, StackMode, Window, WindowClass,
};
use x11rb::wrapper::ConnectionExt as _;

use crate::font;
use crate::ui::{ACCENT, BG, BG_CARD, DANGER, FG, MUTED};

/// Physical top inset reserved for the A26 camera cutout and Moon status UI.
pub const HEIGHT: u16 = 124;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    wifi_connected: bool,
    battery_percent: Option<u8>,
    battery_charging: bool,
}

pub struct StatusBarSurface {
    pub window: Window,
    pixmap: Pixmap,
    gc: Gcontext,
    width: u16,
    mapped: bool,
    redraw: bool,
    last: Option<Snapshot>,
}

impl StatusBarSurface {
    pub fn create<C: Connection>(
        conn: &C,
        root: Window,
        depth: u8,
        width: u16,
    ) -> Result<Self, Box<dyn Error>> {
        let window = conn.generate_id()?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            width,
            HEIGHT,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(BG)
                .override_redirect(1)
                .event_mask(EventMask::EXPOSURE),
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"moon-status",
        )?;
        let pixmap = conn.generate_id()?;
        conn.create_pixmap(depth, pixmap, window, width, HEIGHT)?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, pixmap, &CreateGCAux::new().graphics_exposures(0))?;
        Ok(Self {
            window,
            pixmap,
            gc,
            width,
            mapped: false,
            redraw: true,
            last: None,
        })
    }

    pub fn request_redraw(&mut self) {
        self.redraw = true;
    }

    pub fn request_raise(&mut self) {
        if self.mapped {
            self.redraw = true;
        }
    }

    pub fn sync<C: Connection>(
        &mut self,
        conn: &C,
        visible: bool,
        wifi_connected: bool,
        battery_percent: Option<u8>,
        battery_charging: bool,
    ) -> Result<(), Box<dyn Error>> {
        if !visible {
            if self.mapped {
                conn.unmap_window(self.window)?;
                self.mapped = false;
            }
            return Ok(());
        }

        let snapshot = Snapshot {
            wifi_connected,
            battery_percent: battery_percent.map(|value| value.min(100)),
            battery_charging,
        };
        let newly_mapped = !self.mapped;
        if newly_mapped {
            conn.map_window(self.window)?;
            self.mapped = true;
            self.redraw = true;
        }
        let needs_present = self.redraw || self.last != Some(snapshot);
        if newly_mapped || needs_present {
            conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
        }
        if needs_present {
            self.render(conn, snapshot)?;
            self.redraw = false;
            self.last = Some(snapshot);
        }
        Ok(())
    }

    fn render<C: Connection>(&self, conn: &C, snapshot: Snapshot) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            BG,
            Rectangle {
                x: 0,
                y: 0,
                width: self.width,
                height: HEIGHT,
            },
        )?;

        self.text(conn, "MOON", 64, 34, 6, FG)?;

        let wifi_color = if snapshot.wifi_connected {
            ACCENT
        } else {
            MUTED
        };
        self.text(conn, "WIFI", 680, 48, 3, wifi_color)?;
        self.fill(
            conn,
            if snapshot.wifi_connected {
                ACCENT
            } else {
                DANGER
            },
            Rectangle {
                x: 770,
                y: 53,
                width: 14,
                height: 14,
            },
        )?;
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 816,
                y: 36,
                width: 1,
                height: 52,
            },
        )?;

        let battery_text = snapshot
            .battery_percent
            .map_or_else(|| "--%".to_owned(), |value| format!("{value}%"));
        let battery_x = 962_i16;
        let text_width = font::text_width(&battery_text, 4);
        self.text(
            conn,
            &battery_text,
            battery_x - 18 - text_width as i16,
            43,
            4,
            FG,
        )?;
        self.outline(
            conn,
            MUTED,
            Rectangle {
                x: battery_x,
                y: 43,
                width: 44,
                height: 28,
            },
            2,
        )?;
        self.fill(
            conn,
            MUTED,
            Rectangle {
                x: battery_x + 44,
                y: 51,
                width: 8,
                height: 12,
            },
        )?;
        if let Some(percent) = snapshot.battery_percent {
            let fill_width = u16::from(percent) * 36 / 100;
            if fill_width > 0 {
                self.fill(
                    conn,
                    if percent <= 20 { DANGER } else { ACCENT },
                    Rectangle {
                        x: battery_x + 4,
                        y: 47,
                        width: fill_width,
                        height: 20,
                    },
                )?;
            }
        }
        if snapshot.battery_charging {
            for rectangle in [
                Rectangle {
                    x: battery_x + 19,
                    y: 46,
                    width: 8,
                    height: 9,
                },
                Rectangle {
                    x: battery_x + 14,
                    y: 54,
                    width: 13,
                    height: 7,
                },
                Rectangle {
                    x: battery_x + 14,
                    y: 60,
                    width: 8,
                    height: 9,
                },
            ] {
                self.fill(conn, FG, rectangle)?;
            }
        }
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 0,
                y: (HEIGHT - 1) as i16,
                width: self.width,
                height: 1,
            },
        )?;
        conn.copy_area(
            self.pixmap,
            self.window,
            self.gc,
            0,
            0,
            0,
            0,
            self.width,
            HEIGHT,
        )?;
        Ok(())
    }

    fn text<C: Connection>(
        &self,
        conn: &C,
        text: &str,
        x: i16,
        y: i16,
        scale: u16,
        color: u32,
    ) -> Result<(), Box<dyn Error>> {
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
        let right = rectangle.x + rectangle.width as i16 - thickness as i16;
        let bottom = rectangle.y + rectangle.height as i16 - thickness as i16;
        for line in [
            Rectangle {
                height: thickness,
                ..rectangle
            },
            Rectangle {
                y: bottom,
                height: thickness,
                ..rectangle
            },
            Rectangle {
                width: thickness,
                ..rectangle
            },
            Rectangle {
                x: right,
                width: thickness,
                ..rectangle
            },
        ] {
            self.fill(conn, color, line)?;
        }
        Ok(())
    }

    pub fn destroy<C: Connection>(&self, conn: &C) {
        let _ = conn.destroy_window(self.window);
        let _ = conn.free_pixmap(self.pixmap);
        let _ = conn.free_gc(self.gc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_bar_clears_the_cutout_without_consuming_app_height_twice() {
        assert_eq!(HEIGHT, 124);
        assert_eq!(2340_u16.saturating_sub(HEIGHT), 2216);
        assert_eq!(1520_u16.saturating_sub(HEIGHT), 1396);
    }
}
