use std::error::Error;

use serde::Serialize;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeGCAux, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux,
    EventMask, Gcontext, InputFocus, KEY_PRESS_EVENT, KEY_RELEASE_EVENT, Keycode, Keysym, PropMode,
    Rectangle, StackMode, Window, WindowClass,
};
use x11rb::protocol::xtest::{self, ConnectionExt as _};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

use crate::font;
use crate::ui::{ACCENT, ACCENT_2, BG, BG_CARD, MUTED};

// Apple does not publish fixed software-keyboard frames. These dimensions use
// the standard 390-point iPhone portrait keyboard as a proportional reference
// and scale to Moon's 1080-pixel-wide display. The final 180 physical pixels
// remain Moon's app-close start zone, analogous to the iPhone bottom safe area.
pub const KEYBOARD_HEIGHT: u16 = 820;
pub const CLOSE_START_ZONE_HEIGHT: u16 = 180;
const REFERENCE_KEY_AREA_HEIGHT: u16 = KEYBOARD_HEIGHT - CLOSE_START_ZONE_HEIGHT;
const REFERENCE_SCREEN_WIDTH: u16 = 1080;
const ROW_Y: [u16; 4] = [18, 166, 314, 462];
const ROW_HEIGHT: [u16; 4] = [132, 132, 132, 160];
const KEY_GAP: u16 = 10;

const XK_BACK_SPACE: Keysym = 0xff08;
const XK_RETURN: Keysym = 0xff0d;
const XK_SHIFT_L: Keysym = 0xffe1;
const XK_SHIFT_R: Keysym = 0xffe2;
const XK_MODE_SWITCH: Keysym = 0xff7e;
const XK_ISO_LEVEL3_SHIFT: Keysym = 0xfe03;
const KEY_RELEASE_DELAY_MS: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardPurpose {
    Text,
    Url,
    Search,
    Password,
    Number,
}

impl KeyboardPurpose {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "text" => Some(Self::Text),
            "url" => Some(Self::Url),
            "search" => Some(Self::Search),
            "password" => Some(Self::Password),
            "number" => Some(Self::Number),
            _ => None,
        }
    }

    fn submit_label(self) -> &'static str {
        match self {
            Self::Url => "GO",
            Self::Search => "SEARCH",
            Self::Text | Self::Password | Self::Number => "DONE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyboardLayout {
    Letters,
    Numbers,
    Symbols,
    NumberPad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PublicKeyboardState {
    pub visible: bool,
    pub purpose: Option<KeyboardPurpose>,
    pub shift: bool,
    pub layout: KeyboardLayout,
}

#[derive(Debug)]
pub struct KeyboardState {
    visible: bool,
    purpose: Option<KeyboardPurpose>,
    shift: bool,
    layout: KeyboardLayout,
    redraw: bool,
    raise_pending: bool,
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self {
            visible: false,
            purpose: None,
            shift: false,
            layout: KeyboardLayout::Letters,
            redraw: false,
            raise_pending: false,
        }
    }
}

impl KeyboardState {
    pub fn show(&mut self, purpose: KeyboardPurpose) {
        self.visible = true;
        self.purpose = Some(purpose);
        self.shift = false;
        self.layout = if purpose == KeyboardPurpose::Number {
            KeyboardLayout::NumberPad
        } else {
            KeyboardLayout::Letters
        };
        self.redraw = true;
        self.raise_pending = true;
    }

    pub fn hide(&mut self) -> bool {
        let was_visible = self.visible;
        self.visible = false;
        self.purpose = None;
        self.shift = false;
        self.layout = KeyboardLayout::Letters;
        self.redraw = false;
        self.raise_pending = false;
        was_visible
    }

    pub fn public(&self) -> PublicKeyboardState {
        PublicKeyboardState {
            visible: self.visible,
            purpose: self.purpose,
            shift: self.shift,
            layout: self.layout,
        }
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn purpose(&self) -> Option<KeyboardPurpose> {
        self.purpose
    }

    pub fn layout(&self) -> KeyboardLayout {
        self.layout
    }

    pub fn shift(&self) -> bool {
        self.shift
    }

    pub fn needs_redraw(&self) -> bool {
        self.redraw
    }

    pub fn clear_redraw(&mut self) {
        self.redraw = false;
    }

    pub fn request_redraw(&mut self) {
        if self.visible {
            self.redraw = true;
        }
    }

    pub fn request_raise(&mut self) {
        if self.visible {
            self.raise_pending = true;
        }
    }

    pub fn raise_pending(&self) -> bool {
        self.raise_pending
    }

    pub fn clear_raise(&mut self) {
        self.raise_pending = false;
    }

    /// Apply one key without retaining any text. The returned input exists only
    /// long enough for the caller to emit one XTEST key sequence.
    pub fn activate(&mut self, action: KeyAction) -> KeyboardEffect {
        if !self.visible {
            return KeyboardEffect::None;
        }
        match action {
            KeyAction::Character(mut character) => {
                if self.shift && character.is_ascii_alphabetic() {
                    character.make_ascii_uppercase();
                }
                if self.shift {
                    self.shift = false;
                    self.redraw = true;
                }
                KeyboardEffect::Inject(KeyboardInput::Character(character))
            }
            KeyAction::Shift if self.layout == KeyboardLayout::Letters => {
                self.shift = !self.shift;
                self.redraw = true;
                KeyboardEffect::None
            }
            KeyAction::Shift => KeyboardEffect::None,
            KeyAction::SwitchLayout(layout)
                if self.purpose != Some(KeyboardPurpose::Number)
                    && layout != KeyboardLayout::NumberPad =>
            {
                self.layout = layout;
                self.shift = false;
                self.redraw = true;
                KeyboardEffect::None
            }
            KeyAction::SwitchLayout(_) => KeyboardEffect::None,
            KeyAction::Space => KeyboardEffect::Inject(KeyboardInput::Character(' ')),
            KeyAction::Backspace => KeyboardEffect::Inject(KeyboardInput::Backspace),
            KeyAction::Enter => {
                self.hide();
                KeyboardEffect::Inject(KeyboardInput::Enter)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Character(char),
    Shift,
    SwitchLayout(KeyboardLayout),
    Space,
    Backspace,
    Enter,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyboardInput {
    Character(char),
    Backspace,
    Enter,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyboardEffect {
    None,
    Inject(KeyboardInput),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyRect {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

impl KeyRect {
    fn contains(self, x: i16, y: i16) -> bool {
        let x = i32::from(x);
        let y = i32::from(y);
        x >= i32::from(self.x)
            && x < i32::from(self.x) + i32::from(self.width)
            && y >= i32::from(self.y)
            && y < i32::from(self.y) + i32::from(self.height)
    }

    fn xproto(self) -> Rectangle {
        Rectangle {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardKey {
    pub rect: KeyRect,
    pub label: &'static str,
    pub action: KeyAction,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyboardGeometry {
    pub screen_width: u16,
    pub window_y: i16,
    pub window_height: u16,
    pub key_area_height: u16,
}

impl KeyboardGeometry {
    pub fn new(screen_width: u16, screen_height: u16) -> Self {
        let window_height = screen_height.min(KEYBOARD_HEIGHT);
        let safe_height = window_height.min(CLOSE_START_ZONE_HEIGHT);
        Self {
            screen_width,
            window_y: screen_height.saturating_sub(window_height) as i16,
            window_height,
            key_area_height: window_height.saturating_sub(safe_height),
        }
    }

    pub fn owns_touch(&self, state: &KeyboardState, root_x: i16, root_y: i16) -> bool {
        if !state.is_visible() || root_x < 0 || i32::from(root_x) >= i32::from(self.screen_width) {
            return false;
        }
        let local_y = i32::from(root_y) - i32::from(self.window_y);
        local_y >= 0 && local_y < i32::from(self.key_area_height)
    }

    pub fn app_height(&self) -> u16 {
        self.window_y.max(1) as u16
    }

    pub fn action_at(&self, state: &KeyboardState, root_x: i16, root_y: i16) -> Option<KeyAction> {
        if !self.owns_touch(state, root_x, root_y) {
            return None;
        }
        self.keys(state)
            .get(self.key_index_at(state, root_x, root_y)?)
            .map(|key| key.action)
    }

    pub fn key_index_at(&self, state: &KeyboardState, root_x: i16, root_y: i16) -> Option<usize> {
        if !self.owns_touch(state, root_x, root_y) {
            return None;
        }
        let local_y = root_y.saturating_sub(self.window_y);
        self.keys(state)
            .iter()
            .position(|key| key.rect.contains(root_x, local_y))
    }

    pub fn keys(&self, state: &KeyboardState) -> Vec<KeyboardKey> {
        let mut keys = Vec::with_capacity(48);
        match state.layout() {
            KeyboardLayout::Letters => self.add_letter_keys(state, &mut keys),
            KeyboardLayout::Numbers => self.add_number_keys(state, &mut keys),
            KeyboardLayout::Symbols => self.add_symbol_keys(state, &mut keys),
            KeyboardLayout::NumberPad => self.add_number_pad_keys(&mut keys),
        }
        keys
    }

    fn add_letter_keys(&self, state: &KeyboardState, keys: &mut Vec<KeyboardKey>) {
        self.add_row(
            keys,
            0,
            16,
            &[
                ("Q", KeyAction::Character('q'), 10),
                ("W", KeyAction::Character('w'), 10),
                ("E", KeyAction::Character('e'), 10),
                ("R", KeyAction::Character('r'), 10),
                ("T", KeyAction::Character('t'), 10),
                ("Y", KeyAction::Character('y'), 10),
                ("U", KeyAction::Character('u'), 10),
                ("I", KeyAction::Character('i'), 10),
                ("O", KeyAction::Character('o'), 10),
                ("P", KeyAction::Character('p'), 10),
            ],
        );
        self.add_row(
            keys,
            1,
            64,
            &[
                ("A", KeyAction::Character('a'), 10),
                ("S", KeyAction::Character('s'), 10),
                ("D", KeyAction::Character('d'), 10),
                ("F", KeyAction::Character('f'), 10),
                ("G", KeyAction::Character('g'), 10),
                ("H", KeyAction::Character('h'), 10),
                ("J", KeyAction::Character('j'), 10),
                ("K", KeyAction::Character('k'), 10),
                ("L", KeyAction::Character('l'), 10),
            ],
        );
        self.add_row(
            keys,
            2,
            16,
            &[
                ("SHIFT", KeyAction::Shift, 13),
                ("Z", KeyAction::Character('z'), 10),
                ("X", KeyAction::Character('x'), 10),
                ("C", KeyAction::Character('c'), 10),
                ("V", KeyAction::Character('v'), 10),
                ("B", KeyAction::Character('b'), 10),
                ("N", KeyAction::Character('n'), 10),
                ("M", KeyAction::Character('m'), 10),
                ("DEL", KeyAction::Backspace, 13),
            ],
        );
        self.add_bottom_row(state, keys, KeyboardLayout::Numbers, "123");
    }

    fn add_number_keys(&self, state: &KeyboardState, keys: &mut Vec<KeyboardKey>) {
        self.add_row(
            keys,
            0,
            16,
            &[
                ("1", KeyAction::Character('1'), 10),
                ("2", KeyAction::Character('2'), 10),
                ("3", KeyAction::Character('3'), 10),
                ("4", KeyAction::Character('4'), 10),
                ("5", KeyAction::Character('5'), 10),
                ("6", KeyAction::Character('6'), 10),
                ("7", KeyAction::Character('7'), 10),
                ("8", KeyAction::Character('8'), 10),
                ("9", KeyAction::Character('9'), 10),
                ("0", KeyAction::Character('0'), 10),
            ],
        );
        self.add_row(
            keys,
            1,
            16,
            &[
                ("-", KeyAction::Character('-'), 10),
                ("/", KeyAction::Character('/'), 10),
                (":", KeyAction::Character(':'), 10),
                (";", KeyAction::Character(';'), 10),
                ("(", KeyAction::Character('('), 10),
                (")", KeyAction::Character(')'), 10),
                ("$", KeyAction::Character('$'), 10),
                ("&", KeyAction::Character('&'), 10),
                ("@", KeyAction::Character('@'), 10),
                ("\"", KeyAction::Character('"'), 10),
            ],
        );
        self.add_row(
            keys,
            2,
            16,
            &[
                ("#+=", KeyAction::SwitchLayout(KeyboardLayout::Symbols), 16),
                (".", KeyAction::Character('.'), 10),
                (",", KeyAction::Character(','), 10),
                ("?", KeyAction::Character('?'), 10),
                ("!", KeyAction::Character('!'), 10),
                ("'", KeyAction::Character('\''), 10),
                ("DEL", KeyAction::Backspace, 16),
            ],
        );
        self.add_bottom_row(state, keys, KeyboardLayout::Letters, "ABC");
    }

    fn add_symbol_keys(&self, state: &KeyboardState, keys: &mut Vec<KeyboardKey>) {
        self.add_row(
            keys,
            0,
            16,
            &[
                ("[", KeyAction::Character('['), 10),
                ("]", KeyAction::Character(']'), 10),
                ("{", KeyAction::Character('{'), 10),
                ("}", KeyAction::Character('}'), 10),
                ("#", KeyAction::Character('#'), 10),
                ("%", KeyAction::Character('%'), 10),
                ("^", KeyAction::Character('^'), 10),
                ("*", KeyAction::Character('*'), 10),
                ("+", KeyAction::Character('+'), 10),
                ("=", KeyAction::Character('='), 10),
            ],
        );
        self.add_row(
            keys,
            1,
            64,
            &[
                ("_", KeyAction::Character('_'), 10),
                ("\\", KeyAction::Character('\\'), 10),
                ("|", KeyAction::Character('|'), 10),
                ("~", KeyAction::Character('~'), 10),
                ("<", KeyAction::Character('<'), 10),
                (">", KeyAction::Character('>'), 10),
                ("$", KeyAction::Character('$'), 10),
                ("&", KeyAction::Character('&'), 10),
                ("@", KeyAction::Character('@'), 10),
            ],
        );
        self.add_row(
            keys,
            2,
            16,
            &[
                ("123", KeyAction::SwitchLayout(KeyboardLayout::Numbers), 16),
                (".", KeyAction::Character('.'), 10),
                (",", KeyAction::Character(','), 10),
                ("?", KeyAction::Character('?'), 10),
                ("!", KeyAction::Character('!'), 10),
                ("'", KeyAction::Character('\''), 10),
                ("DEL", KeyAction::Backspace, 16),
            ],
        );
        self.add_bottom_row(state, keys, KeyboardLayout::Letters, "ABC");
    }

    fn add_number_pad_keys(&self, keys: &mut Vec<KeyboardKey>) {
        for (row, definitions) in [
            [
                ("1", KeyAction::Character('1'), 10),
                ("2", KeyAction::Character('2'), 10),
                ("3", KeyAction::Character('3'), 10),
            ],
            [
                ("4", KeyAction::Character('4'), 10),
                ("5", KeyAction::Character('5'), 10),
                ("6", KeyAction::Character('6'), 10),
            ],
            [
                ("7", KeyAction::Character('7'), 10),
                ("8", KeyAction::Character('8'), 10),
                ("9", KeyAction::Character('9'), 10),
            ],
            [
                (".", KeyAction::Character('.'), 10),
                ("0", KeyAction::Character('0'), 10),
                ("DEL", KeyAction::Backspace, 10),
            ],
        ]
        .into_iter()
        .enumerate()
        {
            self.add_row(keys, row, 132, &definitions);
        }
    }

    fn add_bottom_row(
        &self,
        state: &KeyboardState,
        keys: &mut Vec<KeyboardKey>,
        switch_to: KeyboardLayout,
        switch_label: &'static str,
    ) {
        let submit = state
            .purpose()
            .unwrap_or(KeyboardPurpose::Text)
            .submit_label();
        if state.purpose() == Some(KeyboardPurpose::Url) {
            self.add_row(
                keys,
                3,
                16,
                &[
                    (switch_label, KeyAction::SwitchLayout(switch_to), 18),
                    ("SPACE", KeyAction::Space, 48),
                    (".", KeyAction::Character('.'), 12),
                    (submit, KeyAction::Enter, 22),
                ],
            );
        } else {
            self.add_row(
                keys,
                3,
                16,
                &[
                    (switch_label, KeyAction::SwitchLayout(switch_to), 20),
                    ("SPACE", KeyAction::Space, 60),
                    (submit, KeyAction::Enter, 20),
                ],
            );
        }
    }

    fn add_row(
        &self,
        output: &mut Vec<KeyboardKey>,
        row: usize,
        reference_margin: u16,
        definitions: &[(&'static str, KeyAction, u16)],
    ) {
        if definitions.is_empty() || self.key_area_height == 0 {
            return;
        }
        let scale_y = |value: u16| {
            u32::from(value) * u32::from(self.key_area_height)
                / u32::from(REFERENCE_KEY_AREA_HEIGHT)
        };
        let y = scale_y(ROW_Y[row]) as i16;
        let height = scale_y(ROW_HEIGHT[row]) as u16;
        let scale_x = |value: u16| {
            u32::from(value) * u32::from(self.screen_width) / u32::from(REFERENCE_SCREEN_WIDTH)
        };
        let margin = (scale_x(reference_margin) as u16).min(self.screen_width / 3);
        let gap = (scale_x(KEY_GAP) as u16).max(1);
        let gaps = gap.saturating_mul(definitions.len().saturating_sub(1) as u16);
        let available = self
            .screen_width
            .saturating_sub(margin.saturating_mul(2))
            .saturating_sub(gaps);
        let total_weight = definitions
            .iter()
            .map(|definition| u32::from(definition.2))
            .sum::<u32>()
            .max(1);
        let mut x = margin;
        let mut remaining_width = available;
        let mut remaining_weight = total_weight;
        for (index, (label, action, weight)) in definitions.iter().copied().enumerate() {
            let width = if index + 1 == definitions.len() {
                remaining_width
            } else {
                (u32::from(remaining_width) * u32::from(weight) / remaining_weight) as u16
            };
            output.push(KeyboardKey {
                rect: KeyRect {
                    x: x as i16,
                    y,
                    width,
                    height,
                },
                label,
                action,
            });
            x = x.saturating_add(width).saturating_add(gap);
            remaining_width = remaining_width.saturating_sub(width);
            remaining_weight = remaining_weight.saturating_sub(u32::from(weight));
        }
    }
}

pub struct KeyboardSurface {
    pub window: Window,
    back_buffer: u32,
    gc: Gcontext,
    geometry: KeyboardGeometry,
    mapped: bool,
}

impl KeyboardSurface {
    pub fn create<C: Connection>(
        conn: &C,
        root: Window,
        root_depth: u8,
        geometry: KeyboardGeometry,
    ) -> Result<Self, Box<dyn Error>> {
        let window = conn.generate_id()?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            geometry.window_y,
            geometry.screen_width,
            geometry.window_height,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .background_pixel(BG)
                .override_redirect(1)
                .event_mask(EventMask::EXPOSURE | EventMask::STRUCTURE_NOTIFY),
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            x11rb::protocol::xproto::AtomEnum::WM_NAME,
            x11rb::protocol::xproto::AtomEnum::STRING,
            b"moon-keyboard",
        )?;
        let back_buffer = conn.generate_id()?;
        conn.create_pixmap(
            root_depth,
            back_buffer,
            window,
            geometry.screen_width,
            geometry.window_height,
        )?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0))?;
        Ok(Self {
            window,
            back_buffer,
            gc,
            geometry,
            mapped: false,
        })
    }

    pub fn sync<C: Connection>(
        &mut self,
        conn: &C,
        state: &mut KeyboardState,
        active_app: Option<Window>,
    ) -> Result<(), Box<dyn Error>> {
        let should_map = state.is_visible() && active_app.is_some();
        if !should_map {
            if self.mapped {
                conn.unmap_window(self.window)?;
                self.mapped = false;
                if let Some(window) = active_app {
                    conn.set_input_focus(InputFocus::PARENT, window, CURRENT_TIME)?;
                }
            }
            return Ok(());
        }

        if state.needs_redraw() || !self.mapped {
            self.render(conn, state)?;
            state.clear_redraw();
        }
        if !self.mapped {
            conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
            conn.map_window(self.window)?;
            self.mapped = true;
            state.clear_raise();
        } else if state.raise_pending() {
            conn.configure_window(
                self.window,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            )?;
            state.clear_raise();
        }
        Ok(())
    }

    pub fn destroy<C: Connection>(&self, conn: &C) {
        let _ = conn.destroy_window(self.window);
        let _ = conn.free_pixmap(self.back_buffer);
        let _ = conn.free_gc(self.gc);
    }

    fn render<C: Connection>(&self, conn: &C, state: &KeyboardState) -> Result<(), Box<dyn Error>> {
        self.fill(
            conn,
            BG,
            Rectangle {
                x: 0,
                y: 0,
                width: self.geometry.screen_width,
                height: self.geometry.window_height,
            },
        )?;
        self.fill(
            conn,
            ACCENT_2,
            Rectangle {
                x: 0,
                y: 0,
                width: self.geometry.screen_width,
                height: 2,
            },
        )?;

        for key in self.geometry.keys(state) {
            let outline = match key.action {
                KeyAction::Enter => ACCENT,
                KeyAction::Shift if state.shift() => ACCENT,
                KeyAction::Backspace => MUTED,
                _ => ACCENT_2,
            };
            self.fill(conn, BG_CARD, key.rect.xproto())?;
            self.outline(conn, outline, key.rect.xproto(), 3)?;
            self.center_key_label(conn, key.label, key.rect, outline)?;
        }

        let cue_width = self.geometry.screen_width.min(260);
        self.fill(
            conn,
            MUTED,
            Rectangle {
                x: self.geometry.screen_width.saturating_sub(cue_width) as i16 / 2,
                y: self.geometry.window_height.saturating_sub(70) as i16,
                width: cue_width,
                height: 6,
            },
        )?;
        conn.copy_area(
            self.back_buffer,
            self.window,
            self.gc,
            0,
            0,
            0,
            0,
            self.geometry.screen_width,
            self.geometry.window_height,
        )?;
        conn.flush()?;
        Ok(())
    }

    fn center_key_label<C: Connection>(
        &self,
        conn: &C,
        label: &str,
        rect: KeyRect,
        color: u32,
    ) -> Result<(), Box<dyn Error>> {
        let character_count = label.chars().count().max(1) as u16;
        let width_scale = rect
            .width
            .saturating_sub(16)
            .checked_div(character_count.saturating_mul(6))
            .unwrap_or(1);
        let height_scale = rect.height.saturating_sub(16) / 7;
        let scale = width_scale.min(height_scale).clamp(3, 8);
        let text_width = font::text_width(label, scale);
        let x = i32::from(rect.x) + i32::from(rect.width.saturating_sub(text_width) / 2);
        let y = i32::from(rect.y) + i32::from(rect.height.saturating_sub(7 * scale) / 2);
        let rectangles = font::rectangles(label, x as i16, y as i16, scale);
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
        let thickness = thickness.min(rect.width / 2).min(rect.height / 2);
        let bottom = i32::from(rect.y) + i32::from(rect.height.saturating_sub(thickness));
        let right = i32::from(rect.x) + i32::from(rect.width.saturating_sub(thickness));
        let lines = [
            Rectangle {
                x: rect.x,
                y: rect.y,
                width: rect.width,
                height: thickness,
            },
            Rectangle {
                x: rect.x,
                y: bottom as i16,
                width: rect.width,
                height: thickness,
            },
            Rectangle {
                x: rect.x,
                y: rect.y,
                width: thickness,
                height: rect.height,
            },
            Rectangle {
                x: right as i16,
                y: rect.y,
                width: thickness,
                height: rect.height,
            },
        ];
        conn.change_gc(self.gc, &ChangeGCAux::new().foreground(color))?;
        conn.poly_fill_rectangle(self.back_buffer, self.gc, &lines)?;
        Ok(())
    }
}

pub struct XtestInjector {
    root: Window,
    mapping: Option<ServerKeyboardMapping>,
    min_keycode: Keycode,
    max_keycode: Keycode,
}

impl XtestInjector {
    pub fn query<C: Connection>(
        conn: &C,
        root: Window,
        min_keycode: Keycode,
        max_keycode: Keycode,
    ) -> Result<Self, Box<dyn Error>> {
        let mapping = if conn
            .extension_information(xtest::X11_EXTENSION_NAME)?
            .is_some()
        {
            let version = conn.xtest_get_version(2, 2)?.reply()?;
            eprintln!(
                "XTEST keyboard injection ready version={}.{}",
                version.major_version, version.minor_version
            );
            Some(ServerKeyboardMapping::query(
                conn,
                min_keycode,
                max_keycode,
            )?)
        } else {
            eprintln!("XTEST unavailable; on-screen keyboard input is disabled");
            None
        };
        Ok(Self {
            root,
            mapping,
            min_keycode,
            max_keycode,
        })
    }

    pub fn refresh<C: Connection>(&mut self, conn: &C) -> Result<(), Box<dyn Error>> {
        if self.mapping.is_some() {
            self.mapping = Some(ServerKeyboardMapping::query(
                conn,
                self.min_keycode,
                self.max_keycode,
            )?);
        }
        Ok(())
    }

    pub fn inject<C: Connection>(
        &self,
        conn: &C,
        input: KeyboardInput,
        primary_app: Window,
    ) -> Result<(), Box<dyn Error>> {
        let mapping = self.mapping.as_ref().ok_or("XTEST is unavailable")?;
        let focus = conn.get_input_focus()?.reply()?.focus;
        if !self.focus_belongs_to_app(conn, focus, primary_app)? {
            return Err("X focus does not belong to the active app".into());
        }
        let keysym = match input {
            KeyboardInput::Character(character) if character.is_ascii() => character as Keysym,
            KeyboardInput::Character(_) => {
                return Err("non-ASCII keyboard input is unsupported".into());
            }
            KeyboardInput::Backspace => XK_BACK_SPACE,
            KeyboardInput::Enter => XK_RETURN,
        };
        let resolved = mapping
            .resolve(keysym)
            .ok_or("keysym is absent from the X server keyboard mapping")?;

        let mut modifiers = [None, None];
        if resolved.level3 {
            modifiers[0] = mapping.level3_keycode;
        }
        if resolved.shift {
            modifiers[1] = mapping.shift_keycode;
        }
        if (resolved.level3 && modifiers[0].is_none()) || (resolved.shift && modifiers[1].is_none())
        {
            return Err("required modifier is absent from the X server mapping".into());
        }

        let modifiers: Vec<Keycode> = modifiers.into_iter().flatten().collect();
        let mut pressed_modifiers = Vec::with_capacity(modifiers.len());
        for keycode in modifiers.iter().copied() {
            if let Err(error) = self.fake_key(conn, KEY_PRESS_EVENT, keycode) {
                self.release_modifiers_best_effort(conn, &pressed_modifiers);
                return Err(error);
            }
            pressed_modifiers.push(keycode);
        }
        if let Err(error) = self.fake_key(conn, KEY_PRESS_EVENT, resolved.keycode) {
            self.release_modifiers_best_effort(conn, &pressed_modifiers);
            return Err(error);
        }
        let key_release = self.fake_key(conn, KEY_RELEASE_EVENT, resolved.keycode);
        let mut modifier_release_error = None;
        for keycode in pressed_modifiers.iter().copied().rev() {
            if let Err(error) = self.fake_key(conn, KEY_RELEASE_EVENT, keycode)
                && modifier_release_error.is_none()
            {
                modifier_release_error = Some(error);
            }
        }
        conn.flush()?;
        key_release?;
        if let Some(error) = modifier_release_error {
            return Err(error);
        }
        Ok(())
    }

    fn focus_belongs_to_app<C: Connection>(
        &self,
        conn: &C,
        focus: Window,
        primary_app: Window,
    ) -> Result<bool, Box<dyn Error>> {
        if focus == 0 || focus == 1 || primary_app == 0 {
            return Ok(false);
        }
        let mut current = focus;
        for _ in 0..64 {
            if current == primary_app {
                return Ok(true);
            }
            let transient = conn
                .get_property(
                    false,
                    current,
                    AtomEnum::WM_TRANSIENT_FOR,
                    AtomEnum::WINDOW,
                    0,
                    1,
                )?
                .reply()?;
            if transient.value32().and_then(|mut values| values.next()) == Some(primary_app) {
                return Ok(true);
            }
            let tree = conn.query_tree(current)?.reply()?;
            if tree.parent == primary_app {
                return Ok(true);
            }
            if tree.parent == 0 || tree.parent == self.root || tree.parent == current {
                return Ok(false);
            }
            current = tree.parent;
        }
        Ok(false)
    }

    fn release_modifiers_best_effort<C: Connection>(&self, conn: &C, modifiers: &[Keycode]) {
        for keycode in modifiers.iter().copied().rev() {
            let _ = self.fake_key(conn, KEY_RELEASE_EVENT, keycode);
        }
        let _ = conn.flush();
    }

    fn fake_key<C: Connection>(
        &self,
        conn: &C,
        event_type: u8,
        keycode: Keycode,
    ) -> Result<(), Box<dyn Error>> {
        let delay = if event_type == KEY_RELEASE_EVENT {
            KEY_RELEASE_DELAY_MS
        } else {
            CURRENT_TIME
        };
        conn.xtest_fake_input(event_type, keycode, delay, self.root, 0, 0, 0)?
            .check()?;
        Ok(())
    }
}

struct ServerKeyboardMapping {
    min_keycode: Keycode,
    keysyms_per_keycode: usize,
    keysyms: Vec<Keysym>,
    shift_keycode: Option<Keycode>,
    level3_keycode: Option<Keycode>,
}

impl ServerKeyboardMapping {
    fn query<C: Connection>(
        conn: &C,
        min_keycode: Keycode,
        max_keycode: Keycode,
    ) -> Result<Self, Box<dyn Error>> {
        let count = u16::from(max_keycode)
            .saturating_sub(u16::from(min_keycode))
            .saturating_add(1);
        let count = u8::try_from(count).map_err(|_| "invalid X keyboard keycode range")?;
        let keyboard = conn.get_keyboard_mapping(min_keycode, count)?.reply()?;
        let modifiers = conn.get_modifier_mapping()?.reply()?;
        let mut mapping = Self {
            min_keycode,
            keysyms_per_keycode: usize::from(keyboard.keysyms_per_keycode),
            keysyms: keyboard.keysyms,
            shift_keycode: None,
            level3_keycode: None,
        };

        let keycodes_per_modifier = modifiers.keycodes.len() / 8;
        let shift_candidates = modifiers.keycodes.iter().take(keycodes_per_modifier);
        mapping.shift_keycode = shift_candidates
            .clone()
            .copied()
            .find(|keycode| {
                *keycode != 0 && mapping.keycode_has_any(*keycode, &[XK_SHIFT_L, XK_SHIFT_R])
            })
            .or_else(|| shift_candidates.copied().find(|keycode| *keycode != 0));
        mapping.level3_keycode = modifiers.keycodes.iter().copied().find(|keycode| {
            *keycode != 0
                && mapping.keycode_has_any(*keycode, &[XK_MODE_SWITCH, XK_ISO_LEVEL3_SHIFT])
        });
        Ok(mapping)
    }

    fn resolve(&self, wanted: Keysym) -> Option<ResolvedKey> {
        let mut best: Option<(u8, ResolvedKey)> = None;
        if self.keysyms_per_keycode == 0 {
            return None;
        }
        for (key_index, symbols) in self.keysyms.chunks(self.keysyms_per_keycode).enumerate() {
            let keycode = u16::from(self.min_keycode).saturating_add(key_index as u16);
            let Ok(keycode) = u8::try_from(keycode) else {
                continue;
            };
            for (level, keysym) in symbols.iter().copied().take(4).enumerate() {
                if keysym != wanted {
                    continue;
                }
                let resolved = ResolvedKey {
                    keycode,
                    shift: level % 2 == 1,
                    level3: level >= 2,
                };
                let score = u8::from(resolved.shift) + 2 * u8::from(resolved.level3);
                if best.is_none_or(|(best_score, _)| score < best_score) {
                    best = Some((score, resolved));
                }
            }
        }
        best.map(|(_, resolved)| resolved)
    }

    fn keycode_has_any(&self, keycode: Keycode, wanted: &[Keysym]) -> bool {
        let Some(index) = usize::from(keycode)
            .checked_sub(usize::from(self.min_keycode))
            .and_then(|index| index.checked_mul(self.keysyms_per_keycode))
        else {
            return false;
        };
        self.keysyms
            .get(index..index.saturating_add(self.keysyms_per_keycode))
            .is_some_and(|symbols| symbols.iter().any(|symbol| wanted.contains(symbol)))
    }
}

#[derive(Clone, Copy)]
struct ResolvedKey {
    keycode: Keycode,
    shift: bool,
    level3: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible(purpose: KeyboardPurpose) -> KeyboardState {
        let mut state = KeyboardState::default();
        state.show(purpose);
        state
    }

    #[test]
    fn hit_testing_uses_the_rendered_key_rectangles() {
        let geometry = KeyboardGeometry::new(1080, 2340);
        let state = visible(KeyboardPurpose::Url);
        for key in geometry.keys(&state) {
            let root_x = key.rect.x + (key.rect.width / 2) as i16;
            let root_y = geometry.window_y + key.rect.y + (key.rect.height / 2) as i16;
            assert_eq!(geometry.action_at(&state, root_x, root_y), Some(key.action));
        }
    }

    #[test]
    fn close_start_zone_contains_no_keys_and_is_not_consumed() {
        let geometry = KeyboardGeometry::new(1080, 2340);
        let state = visible(KeyboardPurpose::Text);
        let close_zone_y = 2340 - CLOSE_START_ZONE_HEIGHT as i16;
        assert!(
            geometry
                .keys(&state)
                .iter()
                .all(|key| i32::from(geometry.window_y)
                    + i32::from(key.rect.y)
                    + i32::from(key.rect.height)
                    <= i32::from(close_zone_y))
        );
        assert!(!geometry.owns_touch(&state, 540, close_zone_y));
        assert_eq!(geometry.action_at(&state, 540, 2339), None);
    }

    #[test]
    fn url_layout_has_contextual_submit_and_required_punctuation() {
        let geometry = KeyboardGeometry::new(1080, 2340);
        let mut state = visible(KeyboardPurpose::Url);
        let mut keys = geometry.keys(&state);
        assert!(
            keys.iter()
                .any(|key| key.label == "GO" && key.action == KeyAction::Enter)
        );
        assert!(
            keys.iter()
                .any(|key| key.action == KeyAction::Character('.'))
        );
        state.activate(KeyAction::SwitchLayout(KeyboardLayout::Numbers));
        keys.extend(geometry.keys(&state));
        state.activate(KeyAction::SwitchLayout(KeyboardLayout::Symbols));
        keys.extend(geometry.keys(&state));
        for character in ['.', '/', ':', '-', '_'] {
            assert!(
                keys.iter()
                    .any(|key| key.action == KeyAction::Character(character))
            );
        }
    }

    #[test]
    fn submit_labels_match_the_requested_purpose() {
        let geometry = KeyboardGeometry::new(1080, 2340);
        for (purpose, expected) in [
            (KeyboardPurpose::Text, "DONE"),
            (KeyboardPurpose::Url, "GO"),
            (KeyboardPurpose::Search, "SEARCH"),
            (KeyboardPurpose::Password, "DONE"),
        ] {
            let state = visible(purpose);
            assert!(
                geometry
                    .keys(&state)
                    .iter()
                    .any(|key| key.action == KeyAction::Enter && key.label == expected),
                "missing submit label for {purpose:?}"
            );
        }
    }

    #[test]
    fn shift_is_one_shot_and_number_starts_on_number_pad() {
        let mut state = visible(KeyboardPurpose::Text);
        assert!(matches!(
            state.activate(KeyAction::Shift),
            KeyboardEffect::None
        ));
        assert!(state.shift());
        assert!(matches!(
            state.activate(KeyAction::Character('a')),
            KeyboardEffect::Inject(KeyboardInput::Character('A'))
        ));
        assert!(!state.shift());

        state.show(KeyboardPurpose::Number);
        assert_eq!(state.layout(), KeyboardLayout::NumberPad);
        state.activate(KeyAction::SwitchLayout(KeyboardLayout::Letters));
        assert_eq!(state.layout(), KeyboardLayout::NumberPad);
    }

    #[test]
    fn letter_rows_follow_iphone_us_portrait_stagger() {
        let geometry = KeyboardGeometry::new(1080, 2340);
        let state = visible(KeyboardPurpose::Text);
        let keys = geometry.keys(&state);
        let rows: Vec<Vec<&KeyboardKey>> = ROW_Y
            .iter()
            .map(|y| keys.iter().filter(|key| key.rect.y == *y as i16).collect())
            .collect();

        assert_eq!(
            rows[0].iter().map(|key| key.label).collect::<String>(),
            "QWERTYUIOP"
        );
        assert_eq!(
            rows[1].iter().map(|key| key.label).collect::<String>(),
            "ASDFGHJKL"
        );
        assert_eq!(rows[2].len(), 9);
        assert_eq!(rows[2][0].label, "SHIFT");
        assert_eq!(rows[2][8].label, "DEL");
        assert_eq!(
            rows[3].iter().map(|key| key.label).collect::<Vec<_>>(),
            ["123", "SPACE", "DONE"]
        );
        assert!(rows[1][0].rect.x > rows[0][0].rect.x);
        assert!(rows[2][0].rect.width > rows[2][1].rect.width);
        assert!(rows[2][8].rect.width > rows[2][7].rect.width);
    }

    #[test]
    fn number_and_symbol_pages_follow_iphone_key_order() {
        let geometry = KeyboardGeometry::new(1080, 2340);
        let mut state = visible(KeyboardPurpose::Text);
        state.activate(KeyAction::SwitchLayout(KeyboardLayout::Numbers));
        let numbers = geometry.keys(&state);
        assert_eq!(
            numbers
                .iter()
                .take(10)
                .map(|key| key.label)
                .collect::<String>(),
            "1234567890"
        );
        assert!(numbers.iter().any(|key| {
            key.label == "#+=" && key.action == KeyAction::SwitchLayout(KeyboardLayout::Symbols)
        }));

        state.activate(KeyAction::SwitchLayout(KeyboardLayout::Symbols));
        let symbols = geometry.keys(&state);
        assert_eq!(
            symbols
                .iter()
                .take(10)
                .map(|key| key.label)
                .collect::<String>(),
            "[]{}#%^*+="
        );
        assert!(symbols.iter().any(|key| {
            key.label == "123" && key.action == KeyAction::SwitchLayout(KeyboardLayout::Numbers)
        }));
    }

    #[test]
    fn server_mapping_selects_keycodes_and_shift_by_keysym_column() {
        let mapping = ServerKeyboardMapping {
            min_keycode: 8,
            keysyms_per_keycode: 2,
            keysyms: vec!['a' as Keysym, 'A' as Keysym, '1' as Keysym, '!' as Keysym],
            shift_keycode: Some(50),
            level3_keycode: None,
        };
        let lower = mapping.resolve('a' as Keysym).unwrap();
        assert_eq!(lower.keycode, 8);
        assert!(!lower.shift);
        let upper = mapping.resolve('A' as Keysym).unwrap();
        assert_eq!(upper.keycode, 8);
        assert!(upper.shift);
        let symbol = mapping.resolve('!' as Keysym).unwrap();
        assert_eq!(symbol.keycode, 9);
        assert!(symbol.shift);
    }
}
