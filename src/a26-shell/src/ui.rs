use std::error::Error;
use std::fs;
use std::time::Instant;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    ChangeGCAux, ConnectionExt as _, Gcontext, ImageFormat, Rectangle, Window,
};

use crate::font;
use crate::model::{ShellState, View};

const BG: u32 = 0x0b1020;
const BG_CARD: u32 = 0x182235;
const FG: u32 = 0xf5f7fb;
const MUTED: u32 = 0x94a3b8;
const ACCENT: u32 = 0x5eead4;
const ACCENT_2: u32 = 0x60a5fa;
const DANGER: u32 = 0xfb7185;
const APP_ICON_SIZE: u16 = 220;
const APP_ICON_BYTES: usize = APP_ICON_SIZE as usize * APP_ICON_SIZE as usize * 4;
const SYSTEM_ICON_PATH: &str = "/opt/a26-system/share/system-app.bgrx";
const BROWSER_ICON_PATH: &str = "/opt/vimbrowser-a26/share/browser-app.bgrx";

pub fn load_system_icon() -> Option<Vec<u8>> {
    fs::read(SYSTEM_ICON_PATH)
        .ok()
        .filter(|bytes| bytes.len() == APP_ICON_BYTES)
}

pub fn load_browser_icon() -> Option<Vec<u8>> {
    fs::read(BROWSER_ICON_PATH)
        .ok()
        .filter(|bytes| bytes.len() == APP_ICON_BYTES)
}

pub struct Renderer {
    pub window: Window,
    pub back_buffer: u32,
    pub gc: Gcontext,
    pub width: u16,
    pub height: u16,
    pub system_icon: Option<Vec<u8>>,
    pub browser_icon: Option<Vec<u8>>,
}

impl Renderer {
    pub fn render<C: Connection>(
        &self,
        conn: &C,
        state: &ShellState,
    ) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            BG,
            Rectangle {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
        )?;
        if !state.screen_awake {
            // The runtime also turns off the panel backlight, but retain a
            // black X framebuffer as a safe fallback.
            self.present(conn)?;
            return Ok(());
        }
        match state.view {
            View::Locked => self.render_lock(conn, state)?,
            View::Launcher => self.render_launcher(conn, state)?,
            View::System | View::Browser if state.app_launching() => {
                self.render_app_launch(conn, state)?
            }
            View::System | View::Browser => {}
        }
        if state
            .volume_overlay_until
            .is_some_and(|deadline| deadline > Instant::now())
        {
            self.render_volume(conn, state.volume)?;
        }
        self.present(conn)?;
        Ok(())
    }

    fn render_lock<C: Connection>(
        &self,
        conn: &C,
        state: &ShellState,
    ) -> Result<(), Box<dyn Error>> {
        self.center_text(conn, "MOON LOCKED", 250, 10, ACCENT)?;
        self.center_text(conn, "ENTER PIN", 390, 6, MUTED)?;

        let dots_width = 6 * 70;
        let first_x = (i32::from(self.width) - dots_width) / 2;
        for index in 0..6 {
            let color = if index < state.pin_input.len() {
                ACCENT
            } else {
                BG_CARD
            };
            self.fill(
                conn,
                color,
                Rectangle {
                    x: (first_x + index as i32 * 70) as i16,
                    y: 530,
                    width: 34,
                    height: 34,
                },
            )?;
        }

        if let Some(deadline) = state.lockout_until {
            let seconds = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
            self.center_text(conn, &format!("WAIT {seconds} SEC"), 640, 6, DANGER)?;
        } else if state.last_action == "unlock_failed" {
            self.center_text(conn, "INCORRECT PIN", 640, 6, DANGER)?;
        }

        for digit in 1_u8..=9 {
            let index = digit - 1;
            let column = index % 3;
            let row = index / 3;
            let (cx, cy) = keypad_center(self.width, column, row);
            self.key_button(conn, cx, cy, &digit.to_string(), ACCENT_2)?;
        }
        let (del_x, bottom_y) = keypad_center(self.width, 0, 3);
        let (zero_x, _) = keypad_center(self.width, 1, 3);
        let (ok_x, _) = keypad_center(self.width, 2, 3);
        self.key_button(conn, del_x, bottom_y, "DEL", MUTED)?;
        self.key_button(conn, zero_x, bottom_y, "0", ACCENT_2)?;
        self.key_button(conn, ok_x, bottom_y, "OK", ACCENT)?;
        self.center_text(conn, "USB DEBUG CONTROL READY", 2190, 4, MUTED)?;
        Ok(())
    }

    fn render_launcher<C: Connection>(
        &self,
        conn: &C,
        state: &ShellState,
    ) -> Result<(), Box<dyn Error>> {
        // Keep the identity and device status on one visual baseline. Moon is
        // deliberately quiet here so the applications remain the focus.
        self.text(conn, "MOON", 64, 92, 6, FG)?;
        self.render_device_status(conn, state)?;

        // A quiet one-pixel divider preserves the palette without turning the
        // header into another card.
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 64,
                y: 184,
                width: self.width - 128,
                height: 1,
            },
        )?;

        let icon = Rectangle {
            x: 64,
            y: 256,
            width: APP_ICON_SIZE,
            height: APP_ICON_SIZE,
        };
        self.app_icon(conn, self.system_icon.as_deref(), icon.x, icon.y, "SYS")?;
        self.outline(
            conn,
            ACCENT_2,
            Rectangle {
                x: icon.x - 1,
                y: icon.y - 1,
                width: icon.width + 2,
                height: icon.height + 2,
            },
            1,
        )?;
        self.centered_in(conn, "SYSTEM", (64, 512, APP_ICON_SIZE), 5, FG)?;

        let browser = Rectangle {
            x: 380,
            y: 256,
            width: APP_ICON_SIZE,
            height: APP_ICON_SIZE,
        };
        self.app_icon(
            conn,
            self.browser_icon.as_deref(),
            browser.x,
            browser.y,
            "WEB",
        )?;
        self.outline(
            conn,
            ACCENT,
            Rectangle {
                x: browser.x - 1,
                y: browser.y - 1,
                width: browser.width + 2,
                height: browser.height + 2,
            },
            1,
        )?;
        self.centered_in(conn, "BROWSER", (380, 512, APP_ICON_SIZE), 5, FG)?;
        Ok(())
    }

    fn render_app_launch<C: Connection>(
        &self,
        conn: &C,
        state: &ShellState,
    ) -> Result<(), Box<dyn Error>> {
        let (name, icon, fallback, accent) = match state.view {
            View::Browser => ("BROWSER", self.browser_icon.as_deref(), "WEB", ACCENT),
            View::System => return Ok(()),
            View::Locked | View::Launcher => return Ok(()),
        };
        let icon_x = self.width.saturating_sub(APP_ICON_SIZE) / 2;
        let icon_y = 720_i16;
        self.app_icon(conn, icon, icon_x as i16, icon_y, fallback)?;
        self.outline(
            conn,
            accent,
            Rectangle {
                x: icon_x.saturating_sub(1) as i16,
                y: icon_y - 1,
                width: APP_ICON_SIZE + 2,
                height: APP_ICON_SIZE + 2,
            },
            1,
        )?;
        self.center_text(conn, &format!("OPENING {name}"), 1010, 6, FG)?;
        self.center_text(conn, "PLEASE WAIT", 1100, 4, MUTED)?;

        let active = (state.app_launch_elapsed().as_millis() / 180 % 3) as i16;
        for index in 0_i16..3 {
            self.fill(
                conn,
                if index == active { accent } else { BG_CARD },
                Rectangle {
                    x: 478 + index * 50,
                    y: 1190,
                    width: 28,
                    height: 8,
                },
            )?;
        }
        Ok(())
    }

    fn render_device_status<C: Connection>(
        &self,
        conn: &C,
        state: &ShellState,
    ) -> Result<(), Box<dyn Error>> {
        let wifi_color = if state.wifi_connected { ACCENT } else { MUTED };
        self.text(conn, "WIFI", 680, 102, 3, wifi_color)?;
        self.fill(
            conn,
            if state.wifi_connected { ACCENT } else { DANGER },
            Rectangle {
                x: 770,
                y: 106,
                width: 14,
                height: 14,
            },
        )?;
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 816,
                y: 88,
                width: 1,
                height: 50,
            },
        )?;

        let battery_text = battery_status_text(state.battery_percent);
        let battery_x = 962_i16;
        let text_width = font::text_width(&battery_text, 4);
        self.text(
            conn,
            &battery_text,
            battery_x - 18 - text_width as i16,
            99,
            4,
            FG,
        )?;
        self.outline(
            conn,
            MUTED,
            Rectangle {
                x: battery_x,
                y: 99,
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
                y: 107,
                width: 8,
                height: 12,
            },
        )?;
        if let Some(percent) = state.battery_percent {
            let fill_width = battery_fill_width(percent);
            if fill_width > 0 {
                self.fill(
                    conn,
                    if percent <= 20 { DANGER } else { ACCENT },
                    Rectangle {
                        x: battery_x + 4,
                        y: 103,
                        width: fill_width,
                        height: 20,
                    },
                )?;
            }
        }
        Ok(())
    }

    fn render_volume<C: Connection>(&self, conn: &C, volume: u8) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 790,
                y: 330,
                width: 240,
                height: 720,
            },
        )?;
        self.centered_in(conn, "VOL", (790, 375, 240), 5, FG)?;
        self.outline(
            conn,
            MUTED,
            Rectangle {
                x: 875,
                y: 475,
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
                x: 883,
                y: (937 - filled) as i16,
                width: 54,
                height: filled,
            },
        )?;
        self.centered_in(conn, &volume.to_string(), (790, 970, 240), 5, ACCENT)?;
        Ok(())
    }

    fn app_icon<C: Connection>(
        &self,
        conn: &C,
        icon: Option<&[u8]>,
        x: i16,
        y: i16,
        fallback: &str,
    ) -> Result<(), Box<dyn Error>> {
        if let Some(icon) = icon {
            conn.put_image(
                ImageFormat::Z_PIXMAP,
                self.back_buffer,
                self.gc,
                APP_ICON_SIZE,
                APP_ICON_SIZE,
                x,
                y,
                0,
                24,
                icon,
            )?;
        } else {
            self.fill(
                conn,
                BG_CARD,
                Rectangle {
                    x,
                    y,
                    width: APP_ICON_SIZE,
                    height: APP_ICON_SIZE,
                },
            )?;
            self.centered_in(conn, fallback, (x, y + 72, APP_ICON_SIZE), 10, FG)?;
        }
        Ok(())
    }

    fn key_button<C: Connection>(
        &self,
        conn: &C,
        cx: i16,
        cy: i16,
        label: &str,
        color: u32,
    ) -> Result<(), Box<dyn Error>> {
        let rect = Rectangle {
            x: cx - 105,
            y: cy - 105,
            width: 210,
            height: 210,
        };
        self.fill(conn, BG_CARD, rect)?;
        self.outline(conn, color, rect, 4)?;
        let scale = if label.len() > 1 { 5 } else { 10 };
        self.centered_in(conn, label, (rect.x, rect.y + 65, rect.width), scale, color)
    }

    fn centered_in<C: Connection>(
        &self,
        conn: &C,
        text: &str,
        bounds: (i16, i16, u16),
        scale: u16,
        color: u32,
    ) -> Result<(), Box<dyn Error>> {
        let (x, y, width) = bounds;
        let text_width = font::text_width(text, scale);
        let offset = width.saturating_sub(text_width) / 2;
        self.text(conn, text, x.saturating_add(offset as i16), y, scale, color)
    }

    fn center_text<C: Connection>(
        &self,
        conn: &C,
        text: &str,
        y: i16,
        scale: u16,
        color: u32,
    ) -> Result<(), Box<dyn Error>> {
        self.centered_in(conn, text, (0, y, self.width), scale, color)
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
        let rectangles = font::rectangles(text, x, y, scale);
        conn.change_gc(self.gc, &ChangeGCAux::new().foreground(color))?;
        if !rectangles.is_empty() {
            conn.poly_fill_rectangle(self.back_buffer, self.gc, &rectangles)?;
        }
        Ok(())
    }

    fn fill<C: Connection>(
        &self,
        conn: &C,
        color: u32,
        rect: Rectangle,
    ) -> Result<(), Box<dyn Error>> {
        conn.change_gc(self.gc, &ChangeGCAux::new().foreground(color))?;
        conn.poly_fill_rectangle(self.back_buffer, self.gc, &[rect])?;
        Ok(())
    }

    fn outline<C: Connection>(
        &self,
        conn: &C,
        color: u32,
        rect: Rectangle,
        thickness: u16,
    ) -> Result<(), Box<dyn Error>> {
        let t = thickness.min(rect.width / 2).min(rect.height / 2);
        let bottom_y = i32::from(rect.y) + i32::from(rect.height.saturating_sub(t));
        let right_x = i32::from(rect.x) + i32::from(rect.width.saturating_sub(t));
        let lines = [
            Rectangle {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: t,
            },
            Rectangle {
                x: rect.x,
                y: bottom_y as i16,
                width: rect.width,
                height: t,
            },
            Rectangle {
                x: rect.x,
                y: rect.y,
                width: t,
                height: rect.height,
            },
            Rectangle {
                x: right_x as i16,
                y: rect.y,
                width: t,
                height: rect.height,
            },
        ];
        conn.change_gc(self.gc, &ChangeGCAux::new().foreground(color))?;
        conn.poly_fill_rectangle(self.back_buffer, self.gc, &lines)?;
        Ok(())
    }

    fn present<C: Connection>(&self, conn: &C) -> Result<(), Box<dyn Error>> {
        // Build the complete frame off-screen and expose it with one server
        // operation. This prevents the 120 Hz panel from scanning out the
        // intermediate background/text/icon drawing requests.
        conn.copy_area(
            self.back_buffer,
            self.window,
            self.gc,
            0,
            0,
            0,
            0,
            self.width,
            self.height,
        )?;
        conn.flush()?;
        Ok(())
    }
}

fn battery_status_text(battery_percent: Option<u8>) -> String {
    battery_percent
        .map(|value| format!("{}%", value.min(100)))
        .unwrap_or_else(|| "--".into())
}

fn battery_fill_width(battery_percent: u8) -> u16 {
    u16::from(battery_percent.min(100)) * 36 / 100
}

pub fn keypad_action_at(width: u16, x: i16, y: i16) -> Option<KeypadAction> {
    for digit in 1_u8..=9 {
        let index = digit - 1;
        let (cx, cy) = keypad_center(width, index % 3, index / 3);
        if inside_button(x, y, cx, cy) {
            return Some(KeypadAction::Digit(digit));
        }
    }
    let (del_x, bottom_y) = keypad_center(width, 0, 3);
    let (zero_x, _) = keypad_center(width, 1, 3);
    let (ok_x, _) = keypad_center(width, 2, 3);
    if inside_button(x, y, del_x, bottom_y) {
        Some(KeypadAction::Backspace)
    } else if inside_button(x, y, zero_x, bottom_y) {
        Some(KeypadAction::Digit(0))
    } else if inside_button(x, y, ok_x, bottom_y) {
        Some(KeypadAction::Submit)
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
pub enum KeypadAction {
    Digit(u8),
    Backspace,
    Submit,
}

pub fn system_app_at(x: i16, y: i16) -> bool {
    (48..=300).contains(&x) && (232..=564).contains(&y)
}

pub fn browser_app_at(x: i16, y: i16) -> bool {
    (364..=616).contains(&x) && (232..=564).contains(&y)
}

fn keypad_center(width: u16, column: u8, row: u8) -> (i16, i16) {
    let x = i32::from(width) * i32::from(column + 1) / 4;
    let y = 830 + i32::from(row) * 285;
    (x as i16, y as i16)
}

fn inside_button(x: i16, y: i16, cx: i16, cy: i16) -> bool {
    (i32::from(x) - i32::from(cx)).abs() <= 120 && (i32::from(y) - i32::from(cy)).abs() <= 120
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_tiles_have_separate_hit_regions() {
        assert!(system_app_at(174, 512));
        assert!(!browser_app_at(174, 512));
        assert!(browser_app_at(490, 512));
        assert!(!system_app_at(490, 512));
    }

    #[test]
    fn battery_status_is_bounded_and_compact() {
        assert_eq!(battery_status_text(Some(87)), "87%");
        assert_eq!(battery_status_text(None), "--");
        assert_eq!(battery_status_text(Some(200)), "100%");
        assert_eq!(battery_fill_width(0), 0);
        assert_eq!(battery_fill_width(50), 18);
        assert_eq!(battery_fill_width(200), 36);
    }
}
