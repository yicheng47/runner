//! Kitty graphics protocol control-data parser, payload base64 decoding, and
//! response / normalized-command encoders.

use std::fmt::Write as _;

use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use base64::{alphabet, Engine as _};

const BASE64_DECODER: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    InvalidFormat,
    Overflow,
    InvalidData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quiet {
    #[default]
    No,
    Ok,
    Failures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Format {
    Rgb,
    #[default]
    Rgba,
    Png,
}

impl Format {
    pub fn code(self) -> u32 {
        match self {
            Self::Rgb => 24,
            Self::Rgba => 32,
            Self::Png => 100,
        }
    }

    pub fn raw_bpp(self) -> Option<usize> {
        match self {
            Self::Rgb => Some(3),
            Self::Rgba => Some(4),
            Self::Png => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Medium {
    #[default]
    Direct,
    File,
    TemporaryFile,
    SharedMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    #[default]
    None,
    Zlib,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorMovement {
    #[default]
    After,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompositionMode {
    #[default]
    AlphaBlend,
    Overwrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnimationAction {
    #[default]
    Invalid,
    Stop,
    RunWait,
    Run,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transmission {
    pub format: Format,
    /// Unknown `f=` values are deferred until execution so the EINVAL response
    /// carries the image ID.
    pub format_unknown: bool,
    pub medium: Medium,
    pub width: u32,
    pub height: u32,
    pub size: u32,
    pub offset: u32,
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub compression: Compression,
    pub more_chunks: bool,
    pub transient: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Display {
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub x_offset: u32,
    pub y_offset: u32,
    pub columns: u32,
    pub rows: u32,
    pub cursor_movement: CursorMovement,
    pub virtual_placement: bool,
    pub parent_id: u32,
    pub parent_placement_id: u32,
    pub horizontal_offset: i32,
    pub vertical_offset: i32,
    pub z: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteAction {
    All {
        delete_images: bool,
    },
    Id {
        delete_images: bool,
        image_id: u32,
        placement_id: u32,
    },
    Newest {
        delete_images: bool,
        image_number: u32,
        placement_id: u32,
    },
    IntersectCursor {
        delete_images: bool,
    },
    AnimationFrames {
        delete_images: bool,
        image_id: u32,
        image_number: u32,
        frame: u32,
    },
    IntersectCell {
        delete_images: bool,
        x: u32,
        y: u32,
    },
    IntersectCellZ {
        delete_images: bool,
        x: u32,
        y: u32,
        z: i32,
    },
    Range {
        delete_images: bool,
        first: u32,
        last: u32,
    },
    Column {
        delete_images: bool,
        x: u32,
    },
    Row {
        delete_images: bool,
        y: u32,
    },
    Z {
        delete_images: bool,
        z: i32,
    },
}

impl DeleteAction {
    pub fn delete_images(&self) -> bool {
        match *self {
            Self::All { delete_images }
            | Self::Id { delete_images, .. }
            | Self::Newest { delete_images, .. }
            | Self::IntersectCursor { delete_images }
            | Self::AnimationFrames { delete_images, .. }
            | Self::IntersectCell { delete_images, .. }
            | Self::IntersectCellZ { delete_images, .. }
            | Self::Range { delete_images, .. }
            | Self::Column { delete_images, .. }
            | Self::Row { delete_images, .. }
            | Self::Z { delete_images, .. } => delete_images,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delete {
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub action: DeleteAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimationFrameLoading {
    pub transmission: Transmission,
    pub x: u32,
    pub y: u32,
    pub create_frame: u32,
    pub edit_frame: u32,
    pub gap_ms: i32,
    pub composition_mode: CompositionMode,
    pub background: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimationControl {
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub action: AnimationAction,
    pub frame: u32,
    pub gap_ms: i32,
    pub current_frame: u32,
    pub loops: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnimationFrameComposition {
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub dest_frame: u32,
    pub source_frame: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub left_edge: u32,
    pub top_edge: u32,
    pub composition_mode: CompositionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Identifiers {
    pub image_id: u32,
    pub image_number: u32,
    pub placement_id: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Control {
    Query(Transmission),
    Transmit(Transmission),
    TransmitAndDisplay {
        transmission: Transmission,
        display: Display,
    },
    Display(Display),
    Delete(Delete),
    TransmitAnimationFrame(AnimationFrameLoading),
    ControlAnimation(AnimationControl),
    ComposeAnimation(AnimationFrameComposition),
}

impl Control {
    pub fn identifiers(&self) -> Identifiers {
        match self {
            Self::Query(t) | Self::Transmit(t) => Identifiers {
                image_id: t.image_id,
                image_number: t.image_number,
                placement_id: t.placement_id,
            },
            Self::TransmitAndDisplay {
                transmission: t, ..
            } => Identifiers {
                image_id: t.image_id,
                image_number: t.image_number,
                placement_id: t.placement_id,
            },
            Self::Display(d) => Identifiers {
                image_id: d.image_id,
                image_number: d.image_number,
                placement_id: d.placement_id,
            },
            Self::Delete(d) => Identifiers {
                image_id: d.image_id,
                image_number: d.image_number,
                placement_id: d.placement_id,
            },
            Self::TransmitAnimationFrame(f) => Identifiers {
                image_id: f.transmission.image_id,
                image_number: f.transmission.image_number,
                placement_id: f.transmission.placement_id,
            },
            Self::ControlAnimation(a) => Identifiers {
                image_id: a.image_id,
                image_number: a.image_number,
                placement_id: a.placement_id,
            },
            Self::ComposeAnimation(c) => Identifiers {
                image_id: c.image_id,
                image_number: c.image_number,
                placement_id: c.placement_id,
            },
        }
    }

    pub fn transmission(&self) -> Option<&Transmission> {
        match self {
            Self::Query(t) | Self::Transmit(t) => Some(t),
            Self::TransmitAndDisplay { transmission, .. } => Some(transmission),
            Self::TransmitAnimationFrame(f) => Some(&f.transmission),
            _ => None,
        }
    }

    pub fn display(&self) -> Option<&Display> {
        match self {
            Self::Display(d) => Some(d),
            Self::TransmitAndDisplay { display, .. } => Some(display),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub control: Control,
    pub quiet: Quiet,
    /// Base64-decoded payload bytes.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Response {
    pub id: u32,
    pub image_number: u32,
    pub placement_id: u32,
    pub message: &'static str,
}

impl Default for Response {
    fn default() -> Self {
        Self {
            id: 0,
            image_number: 0,
            placement_id: 0,
            message: "OK",
        }
    }
}

impl Response {
    pub fn ok(&self) -> bool {
        self.message == "OK"
    }

    pub fn empty(&self) -> bool {
        self.id == 0 && self.image_number == 0
    }

    pub fn encode(&self) -> Option<String> {
        if self.empty() {
            return None;
        }
        let mut out = String::with_capacity(48);
        out.push_str("\x1b_G");
        let mut prior = false;
        if self.id > 0 {
            let _ = write!(out, "i={}", self.id);
            prior = true;
        }
        if self.image_number > 0 {
            if prior {
                out.push(',');
            }
            let _ = write!(out, "I={}", self.image_number);
            prior = true;
        }
        if self.placement_id > 0 {
            if prior {
                out.push(',');
            }
            let _ = write!(out, "p={}", self.placement_id);
        }
        out.push(';');
        out.push_str(self.message);
        out.push_str("\x1b\\");
        Some(out)
    }
}

struct Kv {
    values: [u32; 52],
    present: u64,
}

impl Default for Kv {
    fn default() -> Self {
        Self {
            values: [0; 52],
            present: 0,
        }
    }
}

impl Kv {
    fn index(key: u8) -> Option<usize> {
        match key {
            b'a'..=b'z' => Some((key - b'a') as usize),
            b'A'..=b'Z' => Some(26 + (key - b'A') as usize),
            _ => None,
        }
    }

    fn get(&self, key: u8) -> Option<u32> {
        let idx = Self::index(key)?;
        if self.present & (1u64 << idx) == 0 {
            return None;
        }
        Some(self.values[idx])
    }

    fn put(&mut self, key: u8, value: u32) {
        let Some(idx) = Self::index(key) else {
            return;
        };
        self.values[idx] = value;
        self.present |= 1u64 << idx;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ParseState {
    ControlKey,
    ControlKeyIgnore,
    ControlValue,
    ControlValueIgnore,
    Data,
}

impl Command {
    /// Parse a Kitty graphics command from the raw bytes between `ESC _ G` and `ESC \`.
    pub fn parse(raw: &[u8]) -> Result<Self, ParseError> {
        let mut kv = Kv::default();
        let mut state = ParseState::ControlKey;
        let mut temp = [0u8; 11];
        let mut temp_len: usize = 0;
        let mut current_key: u8 = 0;
        let mut data_start: Option<usize> = None;

        for (idx, &byte) in raw.iter().enumerate() {
            match state {
                ParseState::ControlKey => match byte {
                    b'=' => {
                        if temp_len != 1 {
                            state = ParseState::ControlValueIgnore;
                        } else {
                            current_key = temp[0];
                            state = ParseState::ControlValue;
                        }
                        temp_len = 0;
                    }
                    b';' => {
                        state = ParseState::Data;
                        data_start = Some(idx + 1);
                        break;
                    }
                    _ => {
                        if temp_len >= temp.len() {
                            state = ParseState::ControlKeyIgnore;
                            temp_len = 0;
                        } else {
                            temp[temp_len] = byte;
                            temp_len += 1;
                        }
                    }
                },
                ParseState::ControlKeyIgnore => {
                    if byte == b'=' {
                        state = ParseState::ControlValueIgnore;
                    }
                }
                ParseState::ControlValue => match byte {
                    b',' => {
                        finish_value(&mut kv, current_key, &temp[..temp_len])?;
                        temp_len = 0;
                        state = ParseState::ControlKey;
                    }
                    b';' => {
                        finish_value(&mut kv, current_key, &temp[..temp_len])?;
                        temp_len = 0;
                        state = ParseState::Data;
                        data_start = Some(idx + 1);
                        break;
                    }
                    _ => {
                        if temp_len >= temp.len() {
                            state = ParseState::ControlValueIgnore;
                            temp_len = 0;
                        } else {
                            temp[temp_len] = byte;
                            temp_len += 1;
                        }
                    }
                },
                ParseState::ControlValueIgnore => match byte {
                    b',' => {
                        state = ParseState::ControlKey;
                        temp_len = 0;
                    }
                    b';' => {
                        state = ParseState::Data;
                        data_start = Some(idx + 1);
                        break;
                    }
                    _ => {}
                },
                ParseState::Data => unreachable!(),
            }
        }

        match state {
            ParseState::ControlKey | ParseState::ControlKeyIgnore => {
                return Err(ParseError::InvalidFormat);
            }
            ParseState::ControlValue => {
                finish_value(&mut kv, current_key, &temp[..temp_len])?;
            }
            ParseState::ControlValueIgnore | ParseState::Data => {}
        }

        let action = match kv.get(b'a') {
            Some(v) => u8::try_from(v).map_err(|_| ParseError::InvalidFormat)?,
            None => b't',
        };
        let control = match action {
            b'q' => Control::Query(parse_transmission(&kv)?),
            b't' => Control::Transmit(parse_transmission(&kv)?),
            b'T' => Control::TransmitAndDisplay {
                transmission: parse_transmission(&kv)?,
                display: parse_display(&kv),
            },
            b'p' => Control::Display(parse_display(&kv)),
            b'd' => Control::Delete(parse_delete(&kv)?),
            b'f' => Control::TransmitAnimationFrame(parse_animation_frame(&kv)?),
            b'a' => Control::ControlAnimation(parse_animation_control(&kv)),
            b'c' => Control::ComposeAnimation(parse_animation_compose(&kv)),
            _ => return Err(ParseError::InvalidFormat),
        };

        let quiet = match kv.get(b'q') {
            None | Some(0) => Quiet::No,
            Some(1) => Quiet::Ok,
            Some(_) => Quiet::Failures,
        };

        let data = match data_start {
            Some(start) if start < raw.len() => BASE64_DECODER
                .decode(&raw[start..])
                .map_err(|_| ParseError::InvalidData)?,
            _ => Vec::new(),
        };

        Ok(Self {
            control,
            quiet,
            data,
        })
    }
}

fn finish_value(kv: &mut Kv, key: u8, bytes: &[u8]) -> Result<(), ParseError> {
    if bytes.is_empty() {
        return Err(ParseError::InvalidFormat);
    }
    if bytes.len() == 1 && !bytes[0].is_ascii_digit() {
        kv.put(key, u32::from(bytes[0]));
        return Ok(());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::InvalidFormat)?;
    let value = match key {
        b'z' | b'H' | b'V' => {
            let signed = text.parse::<i32>().map_err(classify_int_error)?;
            signed as u32
        }
        _ => text.parse::<u32>().map_err(classify_int_error)?,
    };
    kv.put(key, value);
    Ok(())
}

fn classify_int_error(error: std::num::ParseIntError) -> ParseError {
    match error.kind() {
        std::num::IntErrorKind::PosOverflow | std::num::IntErrorKind::NegOverflow => {
            ParseError::Overflow
        }
        _ => ParseError::InvalidFormat,
    }
}

fn parse_transmission(kv: &Kv) -> Result<Transmission, ParseError> {
    let mut t = Transmission::default();
    if let Some(v) = kv.get(b'f') {
        match v {
            24 => t.format = Format::Rgb,
            0 | 32 => t.format = Format::Rgba,
            100 => t.format = Format::Png,
            _ => t.format_unknown = true,
        }
    }
    if let Some(v) = kv.get(b't') {
        let c = u8::try_from(v).map_err(|_| ParseError::InvalidFormat)?;
        t.medium = match c {
            b'd' => Medium::Direct,
            b'f' => Medium::File,
            b't' => Medium::TemporaryFile,
            b's' => Medium::SharedMemory,
            _ => return Err(ParseError::InvalidFormat),
        };
    }
    t.width = kv.get(b's').unwrap_or(0);
    t.height = kv.get(b'v').unwrap_or(0);
    t.size = kv.get(b'S').unwrap_or(0);
    t.offset = kv.get(b'O').unwrap_or(0);
    t.image_id = kv.get(b'i').unwrap_or(0);
    t.image_number = kv.get(b'I').unwrap_or(0);
    t.placement_id = kv.get(b'p').unwrap_or(0);
    if let Some(v) = kv.get(b'o') {
        let c = u8::try_from(v).map_err(|_| ParseError::InvalidFormat)?;
        t.compression = match c {
            b'z' => Compression::Zlib,
            _ => return Err(ParseError::InvalidFormat),
        };
    }
    // Local-only media ignore `m`, matching Kitty and Ghostty.
    if t.medium == Medium::Direct {
        t.more_chunks = kv.get(b'm').is_some_and(|v| v > 0);
    }
    t.transient = kv.get(b'N').is_some_and(|v| (v & 1) != 0);
    Ok(t)
}

fn parse_display(kv: &Kv) -> Display {
    Display {
        image_id: kv.get(b'i').unwrap_or(0),
        image_number: kv.get(b'I').unwrap_or(0),
        placement_id: kv.get(b'p').unwrap_or(0),
        x: kv.get(b'x').unwrap_or(0),
        y: kv.get(b'y').unwrap_or(0),
        width: kv.get(b'w').unwrap_or(0),
        height: kv.get(b'h').unwrap_or(0),
        x_offset: kv.get(b'X').unwrap_or(0),
        y_offset: kv.get(b'Y').unwrap_or(0),
        columns: kv.get(b'c').unwrap_or(0),
        rows: kv.get(b'r').unwrap_or(0),
        cursor_movement: if kv.get(b'C') == Some(1) {
            CursorMovement::None
        } else {
            CursorMovement::After
        },
        virtual_placement: kv.get(b'U').is_some_and(|v| v != 0),
        parent_id: kv.get(b'P').unwrap_or(0),
        parent_placement_id: kv.get(b'Q').unwrap_or(0),
        horizontal_offset: kv.get(b'H').unwrap_or(0) as i32,
        vertical_offset: kv.get(b'V').unwrap_or(0) as i32,
        z: kv.get(b'z').unwrap_or(0) as i32,
    }
}

fn parse_delete(kv: &Kv) -> Result<Delete, ParseError> {
    let what = match kv.get(b'd') {
        Some(v) => u8::try_from(v).map_err(|_| ParseError::InvalidFormat)?,
        None => b'a',
    };
    let action = match what {
        b'a' | b'A' => DeleteAction::All {
            delete_images: what == b'A',
        },
        b'i' | b'I' => DeleteAction::Id {
            delete_images: what == b'I',
            image_id: kv.get(b'i').unwrap_or(0),
            placement_id: kv.get(b'p').unwrap_or(0),
        },
        b'n' | b'N' => DeleteAction::Newest {
            delete_images: what == b'N',
            image_number: kv.get(b'I').unwrap_or(0),
            placement_id: kv.get(b'p').unwrap_or(0),
        },
        b'c' | b'C' => DeleteAction::IntersectCursor {
            delete_images: what == b'C',
        },
        b'f' | b'F' => DeleteAction::AnimationFrames {
            delete_images: what == b'F',
            image_id: kv.get(b'i').unwrap_or(0),
            image_number: kv.get(b'I').unwrap_or(0),
            frame: kv.get(b'r').unwrap_or(0),
        },
        b'p' | b'P' => DeleteAction::IntersectCell {
            delete_images: what == b'P',
            x: kv.get(b'x').unwrap_or(0),
            y: kv.get(b'y').unwrap_or(0),
        },
        b'q' | b'Q' => DeleteAction::IntersectCellZ {
            delete_images: what == b'Q',
            x: kv.get(b'x').unwrap_or(0),
            y: kv.get(b'y').unwrap_or(0),
            z: kv.get(b'z').unwrap_or(0) as i32,
        },
        b'r' | b'R' => DeleteAction::Range {
            delete_images: what == b'R',
            first: kv.get(b'x').unwrap_or(0),
            last: kv.get(b'y').unwrap_or(0),
        },
        b'x' | b'X' => DeleteAction::Column {
            delete_images: what == b'X',
            x: kv.get(b'x').unwrap_or(0),
        },
        b'y' | b'Y' => DeleteAction::Row {
            delete_images: what == b'Y',
            y: kv.get(b'y').unwrap_or(0),
        },
        b'z' | b'Z' => DeleteAction::Z {
            delete_images: what == b'Z',
            z: kv.get(b'z').unwrap_or(0) as i32,
        },
        _ => return Err(ParseError::InvalidFormat),
    };
    Ok(Delete {
        image_id: kv.get(b'i').unwrap_or(0),
        image_number: kv.get(b'I').unwrap_or(0),
        placement_id: kv.get(b'p').unwrap_or(0),
        action,
    })
}

fn parse_animation_frame(kv: &Kv) -> Result<AnimationFrameLoading, ParseError> {
    Ok(AnimationFrameLoading {
        transmission: parse_transmission(kv)?,
        x: kv.get(b'x').unwrap_or(0),
        y: kv.get(b'y').unwrap_or(0),
        create_frame: kv.get(b'c').unwrap_or(0),
        edit_frame: kv.get(b'r').unwrap_or(0),
        gap_ms: kv.get(b'z').unwrap_or(0) as i32,
        composition_mode: if kv.get(b'X') == Some(1) {
            CompositionMode::Overwrite
        } else {
            CompositionMode::AlphaBlend
        },
        background: kv.get(b'Y').unwrap_or(0),
    })
}

fn parse_animation_control(kv: &Kv) -> AnimationControl {
    let action = match kv.get(b's') {
        Some(1) => AnimationAction::Stop,
        Some(2) => AnimationAction::RunWait,
        Some(3) => AnimationAction::Run,
        _ => AnimationAction::Invalid,
    };
    AnimationControl {
        image_id: kv.get(b'i').unwrap_or(0),
        image_number: kv.get(b'I').unwrap_or(0),
        placement_id: kv.get(b'p').unwrap_or(0),
        action,
        frame: kv.get(b'r').unwrap_or(0),
        gap_ms: kv.get(b'z').unwrap_or(0) as i32,
        current_frame: kv.get(b'c').unwrap_or(0),
        loops: kv.get(b'v').unwrap_or(0),
    }
}

fn parse_animation_compose(kv: &Kv) -> AnimationFrameComposition {
    AnimationFrameComposition {
        image_id: kv.get(b'i').unwrap_or(0),
        image_number: kv.get(b'I').unwrap_or(0),
        placement_id: kv.get(b'p').unwrap_or(0),
        dest_frame: kv.get(b'c').unwrap_or(0),
        source_frame: kv.get(b'r').unwrap_or(0),
        x: kv.get(b'x').unwrap_or(0),
        y: kv.get(b'y').unwrap_or(0),
        width: kv.get(b'w').unwrap_or(0),
        height: kv.get(b'h').unwrap_or(0),
        left_edge: kv.get(b'X').unwrap_or(0),
        top_edge: kv.get(b'Y').unwrap_or(0),
        composition_mode: if kv.get(b'C').is_some_and(|v| v != 0) {
            CompositionMode::Overwrite
        } else {
            CompositionMode::AlphaBlend
        },
    }
}

/// Encode a self-contained normalized `a=t` or `a=T` transmission command for
/// mirrors and snapshots.
#[allow(clippy::too_many_arguments)]
pub fn encode_normalized_transmission(
    out: &mut Vec<u8>,
    image_id: u32,
    format: Format,
    width: u32,
    height: u32,
    compression: Compression,
    transient: bool,
    display: Option<&Display>,
    payload: &[u8],
) {
    let mut header = String::with_capacity(96);
    let action = if display.is_some() { 'T' } else { 't' };
    let _ = write!(
        header,
        "\x1b_Ga={action},t=d,i={image_id},f={}",
        format.code()
    );
    if format != Format::Png {
        let _ = write!(header, ",s={width},v={height}");
    }
    if compression == Compression::Zlib {
        header.push_str(",o=z");
    }
    if transient {
        header.push_str(",N=1");
    }
    if let Some(d) = display {
        write_display_keys(&mut header, d);
    }
    header.push_str(",q=2;");
    let encoded_len = base64_encoded_len(payload.len());
    out.reserve(header.len() + encoded_len + 2);
    out.extend_from_slice(header.as_bytes());
    let start = out.len();
    out.resize(start + encoded_len, 0);
    let written = base64::engine::general_purpose::STANDARD
        .encode_slice(payload, &mut out[start..])
        .expect("exact base64 buffer length");
    out.truncate(start + written);
    out.extend_from_slice(b"\x1b\\");
}

pub fn encode_normalized_display(out: &mut Vec<u8>, image_id: u32, display: &Display) {
    let mut seq = String::with_capacity(64);
    let _ = write!(seq, "\x1b_Ga=p,i={image_id}");
    write_display_keys(&mut seq, display);
    seq.push_str(",q=2\x1b\\");
    out.extend_from_slice(seq.as_bytes());
}

pub fn encode_normalized_delete(out: &mut Vec<u8>, action: &DeleteAction) {
    let mut seq = String::with_capacity(48);
    seq.push_str("\x1b_Ga=d,d=");
    match *action {
        DeleteAction::All { delete_images } => {
            seq.push(if delete_images { 'A' } else { 'a' });
        }
        DeleteAction::Id {
            delete_images,
            image_id,
            placement_id,
        } => {
            seq.push(if delete_images { 'I' } else { 'i' });
            if image_id > 0 {
                let _ = write!(seq, ",i={image_id}");
            }
            if placement_id > 0 {
                let _ = write!(seq, ",p={placement_id}");
            }
        }
        DeleteAction::Newest {
            delete_images,
            image_number,
            placement_id,
        } => {
            // Callers resolve `n`/`N` into `Id` before normalizing; this arm is a fallback.
            seq.push(if delete_images { 'N' } else { 'n' });
            if image_number > 0 {
                let _ = write!(seq, ",I={image_number}");
            }
            if placement_id > 0 {
                let _ = write!(seq, ",p={placement_id}");
            }
        }
        DeleteAction::IntersectCursor { delete_images } => {
            seq.push(if delete_images { 'C' } else { 'c' });
        }
        DeleteAction::AnimationFrames {
            delete_images,
            image_id,
            image_number,
            frame,
        } => {
            seq.push(if delete_images { 'F' } else { 'f' });
            if image_id > 0 {
                let _ = write!(seq, ",i={image_id}");
            }
            if image_number > 0 {
                let _ = write!(seq, ",I={image_number}");
            }
            if frame > 0 {
                let _ = write!(seq, ",r={frame}");
            }
        }
        DeleteAction::IntersectCell {
            delete_images,
            x,
            y,
        } => {
            seq.push(if delete_images { 'P' } else { 'p' });
            if x > 0 {
                let _ = write!(seq, ",x={x}");
            }
            if y > 0 {
                let _ = write!(seq, ",y={y}");
            }
        }
        DeleteAction::IntersectCellZ {
            delete_images,
            x,
            y,
            z,
        } => {
            seq.push(if delete_images { 'Q' } else { 'q' });
            if x > 0 {
                let _ = write!(seq, ",x={x}");
            }
            if y > 0 {
                let _ = write!(seq, ",y={y}");
            }
            if z != 0 {
                let _ = write!(seq, ",z={z}");
            }
        }
        DeleteAction::Range {
            delete_images,
            first,
            last,
        } => {
            seq.push(if delete_images { 'R' } else { 'r' });
            if first > 0 {
                let _ = write!(seq, ",x={first}");
            }
            if last > 0 {
                let _ = write!(seq, ",y={last}");
            }
        }
        DeleteAction::Column { delete_images, x } => {
            seq.push(if delete_images { 'X' } else { 'x' });
            if x > 0 {
                let _ = write!(seq, ",x={x}");
            }
        }
        DeleteAction::Row { delete_images, y } => {
            seq.push(if delete_images { 'Y' } else { 'y' });
            if y > 0 {
                let _ = write!(seq, ",y={y}");
            }
        }
        DeleteAction::Z { delete_images, z } => {
            seq.push(if delete_images { 'Z' } else { 'z' });
            if z != 0 {
                let _ = write!(seq, ",z={z}");
            }
        }
    }
    seq.push_str(",q=2\x1b\\");
    out.extend_from_slice(seq.as_bytes());
}

pub fn encode_delete_image_by_id(out: &mut Vec<u8>, image_id: u32) {
    let _ = write!(
        ByteStringWriter(out),
        "\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\"
    );
}

pub fn encode_delete_placements_by_id(out: &mut Vec<u8>, image_id: u32) {
    let _ = write!(
        ByteStringWriter(out),
        "\x1b_Ga=d,d=i,i={image_id},q=2\x1b\\"
    );
}

pub fn base64_encoded_len(raw_len: usize) -> usize {
    raw_len.div_ceil(3) * 4
}

pub fn write_display_keys(out: &mut String, d: &Display) {
    if d.placement_id > 0 {
        let _ = write!(out, ",p={}", d.placement_id);
    }
    if d.x > 0 {
        let _ = write!(out, ",x={}", d.x);
    }
    if d.y > 0 {
        let _ = write!(out, ",y={}", d.y);
    }
    if d.width > 0 {
        let _ = write!(out, ",w={}", d.width);
    }
    if d.height > 0 {
        let _ = write!(out, ",h={}", d.height);
    }
    if d.x_offset > 0 {
        let _ = write!(out, ",X={}", d.x_offset);
    }
    if d.y_offset > 0 {
        let _ = write!(out, ",Y={}", d.y_offset);
    }
    if d.columns > 0 {
        let _ = write!(out, ",c={}", d.columns);
    }
    if d.rows > 0 {
        let _ = write!(out, ",r={}", d.rows);
    }
    if d.cursor_movement == CursorMovement::None {
        out.push_str(",C=1");
    }
    if d.virtual_placement {
        out.push_str(",U=1");
    }
    if d.parent_id > 0 {
        let _ = write!(out, ",P={}", d.parent_id);
    }
    if d.parent_placement_id > 0 {
        let _ = write!(out, ",Q={}", d.parent_placement_id);
    }
    if d.horizontal_offset != 0 {
        let _ = write!(out, ",H={}", d.horizontal_offset);
    }
    if d.vertical_offset != 0 {
        let _ = write!(out, ",V={}", d.vertical_offset);
    }
    if d.z != 0 {
        let _ = write!(out, ",z={}", d.z);
    }
}

struct ByteStringWriter<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for ByteStringWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_transmission_query_display_and_delete_vectors() {
        let cmd = Command::parse(b"f=24,s=10,v=20,N=1").unwrap();
        assert_eq!(
            cmd.control,
            Control::Transmit(Transmission {
                format: Format::Rgb,
                width: 10,
                height: 20,
                transient: true,
                ..Default::default()
            })
        );

        let cmd = Command::parse(b"i=31,s=1,v=1,a=q,t=d,f=24;QUFBQQ").unwrap();
        assert_eq!(cmd.data, b"AAAA");
        assert!(matches!(
            cmd.control,
            Control::Query(Transmission {
                image_id: 31,
                width: 1,
                height: 1,
                format: Format::Rgb,
                medium: Medium::Direct,
                ..
            })
        ));

        let cmd = Command::parse(b";QUFBQQ").unwrap();
        assert!(matches!(cmd.control, Control::Transmit(_)));
        assert_eq!(cmd.data, b"AAAA");

        let cmd = Command::parse(b"a=p,U=1,i=31,c=80,r=120,z=-2000000000,H=-1,V=-2").unwrap();
        assert_eq!(
            cmd.control,
            Control::Display(Display {
                image_id: 31,
                columns: 80,
                rows: 120,
                virtual_placement: true,
                z: -2_000_000_000,
                horizontal_offset: -1,
                vertical_offset: -2,
                ..Default::default()
            })
        );

        let cmd = Command::parse(b"a=d,d=p,x=3,y=4").unwrap();
        assert_eq!(
            cmd.control,
            Control::Delete(Delete {
                image_id: 0,
                image_number: 0,
                placement_id: 0,
                action: DeleteAction::IntersectCell {
                    delete_images: false,
                    x: 3,
                    y: 4,
                },
            })
        );
    }

    #[test]
    fn ignores_unknown_long_keys_and_rejects_overflows_and_bad_flags() {
        let cmd = Command::parse(b"f=24,s=10,v=20,hello=world,!=1").unwrap();
        assert_eq!(cmd.control.transmission().unwrap().width, 10);
        assert_eq!(cmd.control.transmission().unwrap().height, 20);

        let cmd = Command::parse(b"f=24,s=10,v=2000000000000000000000000000000000000000").unwrap();
        assert_eq!(cmd.control.transmission().unwrap().height, 0);

        assert_eq!(
            Command::parse(b"a=p,i=10000000000"),
            Err(ParseError::Overflow)
        );
        assert_eq!(
            Command::parse(b"a=p,i=1,z=-9999999999"),
            Err(ParseError::Overflow)
        );
        assert_eq!(
            Command::parse(b"abcdef1234"),
            Err(ParseError::InvalidFormat)
        );
        for bad in [
            b"a=z,i=31".as_slice(),
            b"a=d,d=w,i=31",
            b"a=t,t=q,i=31",
            b"a=t,o=q,i=31",
        ] {
            assert_eq!(Command::parse(bad), Err(ParseError::InvalidFormat));
        }

        // Unknown format is deferred to execution.
        let cmd = Command::parse(b"a=t,f=42,i=31,s=1,v=1").unwrap();
        assert!(cmd.control.transmission().unwrap().format_unknown);

        // Zero format is RGBA; q >= 2 suppresses failures; non-direct ignores m.
        let cmd = Command::parse(b"a=t,f=0,t=t,m=1,q=3").unwrap();
        let t = cmd.control.transmission().unwrap();
        assert_eq!(t.format, Format::Rgba);
        assert!(!t.more_chunks);
        assert_eq!(cmd.quiet, Quiet::Failures);
    }

    #[test]
    fn accepts_unpadded_and_trailing_bit_base64_from_ghostty_vectors() {
        let cmd = Command::parse(b"a=t,t=d,i=1,s=1,v=2;///////////").unwrap();
        assert_eq!(cmd.data.len(), 8);
        let cmd = Command::parse(b"a=t,f=24,s=1,v=1,i=41,p=7,m=1;AA==").unwrap();
        assert_eq!(cmd.data, vec![0]);
    }

    #[test]
    fn response_encoding_matches_ghostty() {
        assert_eq!(Response::default().encode(), None);
        assert_eq!(
            Response {
                id: 4,
                ..Default::default()
            }
            .encode()
            .as_deref(),
            Some("\x1b_Gi=4;OK\x1b\\")
        );
        assert_eq!(
            Response {
                image_number: 4,
                ..Default::default()
            }
            .encode()
            .as_deref(),
            Some("\x1b_GI=4;OK\x1b\\")
        );
        assert_eq!(
            Response {
                id: 12,
                image_number: 4,
                placement_id: 7,
                message: "EINVAL: invalid data",
            }
            .encode()
            .as_deref(),
            Some("\x1b_Gi=12,I=4,p=7;EINVAL: invalid data\x1b\\")
        );
    }
}
