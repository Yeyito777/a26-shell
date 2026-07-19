use std::error::Error;
use std::ffi::CString;
use std::fs;
use std::mem::MaybeUninit;
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
const SYSTEM_ICON_SIZE: u16 = 220;
const SYSTEM_ICON: &[u8] = include_bytes!("../assets/system-app.bgrx");

#[derive(Debug, Clone, Copy)]
struct CpuTimes {
    total: u64,
    idle: u64,
}

#[derive(Debug, Clone)]
pub struct SystemInfo {
    pub kernel: String,
    pub alpine: String,
    pub memory: String,
    pub cpu_usage: Option<u8>,
    pub gpu_usage: Option<u8>,
    pub storage_used: String,
    pub storage_remaining: String,
    previous_cpu: Option<CpuTimes>,
}

impl SystemInfo {
    pub fn collect() -> Self {
        let kernel = read_trimmed("/proc/sys/kernel/osrelease").unwrap_or_else(|| "UNKNOWN".into());
        let alpine = read_trimmed("/etc/alpine-release").unwrap_or_else(|| "UNKNOWN".into());
        let memory = fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|text| {
                text.lines().find_map(|line| {
                    let value = line.strip_prefix("MemTotal:")?.split_whitespace().next()?;
                    let kib: u64 = value.parse().ok()?;
                    Some(format!("{} MB", kib / 1024))
                })
            })
            .unwrap_or_else(|| "UNKNOWN".into());
        let (storage_used, storage_remaining) = storage_usage("/")
            .map(|(used, remaining)| (format_bytes(used), format_bytes(remaining)))
            .unwrap_or_else(|| ("UNKNOWN".into(), "UNKNOWN".into()));
        let mut result = Self {
            kernel,
            alpine,
            memory,
            cpu_usage: None,
            gpu_usage: read_gpu_usage(),
            storage_used,
            storage_remaining,
            previous_cpu: None,
        };
        result.refresh();
        result
    }

    pub fn refresh(&mut self) {
        if let Some(current) = read_cpu_times() {
            if let Some(previous) = self.previous_cpu {
                self.cpu_usage = cpu_usage(previous, current);
            }
            self.previous_cpu = Some(current);
        }
        self.gpu_usage = read_gpu_usage();
        if let Some((used, remaining)) = storage_usage("/") {
            self.storage_used = format_bytes(used);
            self.storage_remaining = format_bytes(remaining);
        }
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    Some(fs::read_to_string(path).ok()?.trim().to_string())
}

fn read_cpu_times() -> Option<CpuTimes> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let mut fields = stat.lines().next()?.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let values: Vec<u64> = fields.map(str::parse).collect::<Result<_, _>>().ok()?;
    if values.len() < 5 {
        return None;
    }
    Some(CpuTimes {
        total: values.iter().copied().sum(),
        idle: values[3].saturating_add(values[4]),
    })
}

fn cpu_usage(previous: CpuTimes, current: CpuTimes) -> Option<u8> {
    let total = current.total.saturating_sub(previous.total);
    if total == 0 {
        return None;
    }
    let idle = current.idle.saturating_sub(previous.idle).min(total);
    let busy = total - idle;
    Some(((busy.saturating_mul(100) + total / 2) / total).min(100) as u8)
}

fn read_gpu_usage() -> Option<u8> {
    read_trimmed("/sys/class/misc/mali0/device/utilization")?
        .split_whitespace()
        .next()?
        .parse::<u16>()
        .ok()
        .map(|value| value.min(100) as u8)
}

fn storage_usage(path: &str) -> Option<(u64, u64)> {
    let path = CString::new(path).ok()?;
    let mut stats = MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a live NUL-terminated C string and `stats` points to
    // writable storage for one `statvfs` result. The result is read only when
    // libc reports success.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: the successful libc call above initialized the complete struct.
    let stats = unsafe { stats.assume_init() };
    let block_size = stats.f_frsize.max(1);
    let total = stats.f_blocks.saturating_mul(block_size);
    let free = stats.f_bfree.saturating_mul(block_size);
    let remaining = stats.f_bavail.saturating_mul(block_size);
    Some((total.saturating_sub(free), remaining))
}

fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    format!("{:.1} GB", bytes as f64 / GIB)
}

fn usage_text(usage: Option<u8>) -> String {
    usage.map_or_else(|| "--%".into(), |value| format!("{value}%"))
}

pub struct Renderer {
    pub window: Window,
    pub gc: Gcontext,
    pub width: u16,
    pub height: u16,
    pub system: SystemInfo,
}

impl Renderer {
    pub fn refresh_system(&mut self) {
        self.system.refresh();
    }

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
            conn.flush()?;
            return Ok(());
        }
        match state.view {
            View::Locked => self.render_lock(conn, state)?,
            View::Launcher => self.render_launcher(conn, state)?,
            View::System => self.render_system(conn, state)?,
        }
        if state
            .volume_overlay_until
            .is_some_and(|deadline| deadline > Instant::now())
        {
            self.render_volume(conn, state.volume)?;
        }
        conn.flush()?;
        Ok(())
    }

    fn render_lock<C: Connection>(
        &self,
        conn: &C,
        state: &ShellState,
    ) -> Result<(), Box<dyn Error>> {
        self.center_text(conn, "A26 LOCKED", 250, 10, ACCENT)?;
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
        self.text(conn, "A26", 64, 88, 8, FG)?;
        self.text(conn, "LINUX", 66, 172, 3, MUTED)?;
        let volume = format!("VOL {}", state.volume);
        let volume_x = self
            .width
            .saturating_sub(64)
            .saturating_sub(font::text_width(&volume, 4));
        self.text(conn, &volume, volume_x as i16, 112, 4, MUTED)?;

        // A quiet one-pixel divider preserves the palette without turning the
        // header into another card.
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 64,
                y: 252,
                width: self.width - 128,
                height: 1,
            },
        )?;
        self.text(conn, "APPS", 64, 322, 4, MUTED)?;

        let icon = Rectangle {
            x: 64,
            y: 402,
            width: SYSTEM_ICON_SIZE,
            height: SYSTEM_ICON_SIZE,
        };
        self.system_icon(conn, icon.x, icon.y)?;
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
        self.centered_in(conn, "SYSTEM", (64, 658, SYSTEM_ICON_SIZE), 5, FG)?;
        Ok(())
    }

    fn render_system<C: Connection>(
        &self,
        conn: &C,
        state: &ShellState,
    ) -> Result<(), Box<dyn Error>> {
        self.text(conn, "SYSTEM", 70, 100, 10, ACCENT)?;
        self.text(conn, &format!("VOLUME {}", state.volume), 70, 260, 5, MUTED)?;
        let cpu = usage_text(self.system.cpu_usage);
        let gpu = usage_text(self.system.gpu_usage);
        self.metric_card(conn, (55, 390), "CPU", &cpu, ACCENT)?;
        self.metric_card(conn, (555, 390), "GPU", &gpu, ACCENT_2)?;
        self.metric_card(conn, (55, 575), "SPACE USED", &self.system.storage_used, FG)?;
        self.metric_card(
            conn,
            (555, 575),
            "SPACE FREE",
            &self.system.storage_remaining,
            FG,
        )?;
        self.card_line(conn, 780, "DEVICE", "SAMSUNG GALAXY A26")?;
        self.card_line(conn, 945, "KERNEL", &self.system.kernel)?;
        self.card_line(
            conn,
            1110,
            "ROOTFS",
            &format!("ALPINE {}", self.system.alpine),
        )?;
        self.card_line(conn, 1275, "MEMORY", &self.system.memory)?;
        self.card_line(
            conn,
            1440,
            "DISPLAY",
            &format!("{}X{} 120 HZ", self.width, self.height),
        )?;
        self.card_line(conn, 1605, "WINDOW MANAGER", "A26 SHELL RUST")?;
        self.card_line(conn, 1770, "CONTROL", "ADB UNIX SOCKET")?;
        self.center_text(conn, "SWIPE UP FROM BELOW TO CLOSE", 2150, 4, MUTED)?;
        self.home_bar(conn)?;
        Ok(())
    }

    fn metric_card<C: Connection>(
        &self,
        conn: &C,
        position: (i16, i16),
        noun: &str,
        value: &str,
        color: u32,
    ) -> Result<(), Box<dyn Error>> {
        let (x, y) = position;
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x,
                y,
                width: 470,
                height: 145,
            },
        )?;
        self.text(conn, noun, x + 28, y + 24, 3, MUTED)?;
        let clipped: String = value.chars().take(12).collect();
        self.text(conn, &clipped, x + 28, y + 73, 6, color)?;
        Ok(())
    }

    fn card_line<C: Connection>(
        &self,
        conn: &C,
        y: i16,
        noun: &str,
        value: &str,
    ) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            BG_CARD,
            Rectangle {
                x: 55,
                y,
                width: self.width - 110,
                height: 145,
            },
        )?;
        self.text(conn, noun, 85, y + 25, 4, MUTED)?;
        let clipped: String = value.chars().take(28).collect();
        self.text(conn, &clipped, 85, y + 78, 4, FG)?;
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

    fn system_icon<C: Connection>(&self, conn: &C, x: i16, y: i16) -> Result<(), Box<dyn Error>> {
        conn.put_image(
            ImageFormat::Z_PIXMAP,
            self.window,
            self.gc,
            SYSTEM_ICON_SIZE,
            SYSTEM_ICON_SIZE,
            x,
            y,
            0,
            24,
            SYSTEM_ICON,
        )?;
        Ok(())
    }

    fn home_bar<C: Connection>(&self, conn: &C) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            FG,
            Rectangle {
                x: (self.width / 2 - 145) as i16,
                y: (self.height - 55) as i16,
                width: 290,
                height: 14,
            },
        )
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
            conn.poly_fill_rectangle(self.window, self.gc, &rectangles)?;
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
        conn.poly_fill_rectangle(self.window, self.gc, &[rect])?;
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
        conn.poly_fill_rectangle(self.window, self.gc, &lines)?;
        Ok(())
    }
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
    (48..=300).contains(&x) && (380..=710).contains(&y)
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
    fn cpu_usage_uses_busy_delta() {
        let previous = CpuTimes {
            total: 1_000,
            idle: 700,
        };
        let current = CpuTimes {
            total: 1_200,
            idle: 780,
        };
        assert_eq!(cpu_usage(previous, current), Some(60));
    }

    #[test]
    fn storage_size_is_human_readable() {
        assert_eq!(format_bytes(16 * 1024 * 1024 * 1024), "16.0 GB");
    }
}
