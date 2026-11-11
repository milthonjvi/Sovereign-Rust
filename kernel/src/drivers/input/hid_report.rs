//! Bounded HID Report-descriptor interpretation and semantic report decoding.
//!
//! This is intentionally an Input-layer parser. USB only retrieves and transports
//! descriptor/report bytes. No allocation, recursion, or unbounded item storage.
use super::event::{GamepadAxis, InputDeviceId, InputEvent};
use super::queue::SpscProducer;
use super::usb_hid_adapter::HidPublishStats;

pub const MAX_HID_INPUT_FIELDS: usize = 64;
pub const MAX_HID_VALUES: usize = 128;
const MAX_COLLECTION_DEPTH: usize = 8;
const MAX_GLOBAL_DEPTH: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HidReportError {
    TruncatedDescriptor,
    MalformedDescriptor,
    CollectionOverflow,
    TooManyFields,
    TooManyValues,
    ArithmeticOverflow,
    MissingReport,
    ReportIdMismatch,
    ReportTooShort,
    InvalidField,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HidInputField {
    pub report_id: u8,
    pub bit_offset: u32,
    pub report_size: u8,
    pub report_count: u8,
    pub value_slot: u16,
    pub usage_page: u16,
    pub usage_min: u16,
    pub usage_max: u16,
    pub usage_list: [u16; 16],
    pub usage_count: u8,
    pub logical_min: i32,
    pub logical_max: i32,
    /// HID Input item flags. Bit 1=Variable, bit 2=Relative.
    pub flags: u8,
    pub application_page: u16,
    pub application_usage: u16,
}
impl HidInputField {
    const EMPTY: Self = Self {
        report_id: 0, bit_offset: 0, report_size: 0, report_count: 0,
        value_slot: 0, usage_page: 0, usage_min: 0, usage_max: 0,
        usage_list: [0; 16], usage_count: 0,
        logical_min: 0, logical_max: 0, flags: 0,
        application_page: 0, application_usage: 0,
    };
    pub const fn is_constant(self) -> bool { self.flags & 1 != 0 }
    pub const fn is_variable(self) -> bool { self.flags & 2 != 0 }
    pub const fn is_relative(self) -> bool { self.flags & 4 != 0 }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct GlobalState {
    usage_page: u16,
    logical_min: i32,
    logical_max: i32,
    report_size: u32,
    report_count: u32,
    report_id: u8,
}
impl GlobalState {
    const EMPTY: Self = Self {
        usage_page: 0, logical_min: 0, logical_max: 0,
        report_size: 0, report_count: 0, report_id: 0,
    };
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LocalState {
    usage_min: Option<u16>,
    usage_max: Option<u16>,
    first_usage: Option<u16>,
    usages: [u16; 16],
    usage_count: u8,
}
impl LocalState {
    const EMPTY: Self = Self { usage_min: None, usage_max: None, first_usage: None, usages: [0; 16], usage_count: 0 };
    fn clear(&mut self) { *self = Self::EMPTY; }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Collection {
    page: u16,
    usage: u16,
}
impl Collection { const EMPTY: Self = Self { page: 0, usage: 0 }; }

/// Fixed-storage descriptor model. Each field represents one HID Main Input
/// item; its report_count values occupy consecutive bits and state slots.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HidReportMap {
    fields: [Option<HidInputField>; MAX_HID_INPUT_FIELDS],
    field_count: u8,
    value_count: u16,
    report_bits: [u32; 256],
}
impl HidReportMap {
    pub const fn empty() -> Self {
        Self {
            fields: [None; MAX_HID_INPUT_FIELDS],
            field_count: 0,
            value_count: 0,
            report_bits: [0; 256],
        }
    }

    pub fn parse(descriptor: &[u8]) -> Result<Self, HidReportError> {
        let mut map = Self::empty();
        let mut global = GlobalState::EMPTY;
        let mut global_stack = [GlobalState::EMPTY; MAX_GLOBAL_DEPTH];
        let mut global_depth = 0usize;
        let mut local = LocalState::EMPTY;
        let mut collections = [Collection::EMPTY; MAX_COLLECTION_DEPTH];
        let mut collection_depth = 0usize;
        let mut application = Collection::EMPTY;
        let mut offset = 0usize;

        while offset < descriptor.len() {
            let prefix = descriptor[offset];
            offset += 1;
            if prefix == 0xFE {
                if descriptor.len().saturating_sub(offset) < 2 {
                    return Err(HidReportError::TruncatedDescriptor);
                }
                let size = descriptor[offset] as usize;
                offset += 2; // data size and long-item tag
                if size > descriptor.len().saturating_sub(offset) {
                    return Err(HidReportError::TruncatedDescriptor);
                }
                offset += size;
                continue;
            }

            let size = match prefix & 3 { 0 => 0, 1 => 1, 2 => 2, _ => 4 };
            if size > descriptor.len().saturating_sub(offset) {
                return Err(HidReportError::TruncatedDescriptor);
            }
            let mut raw = 0u32;
            for i in 0..size { raw |= (descriptor[offset + i] as u32) << (i * 8); }
            offset += size;
            let item_type = (prefix >> 2) & 3;
            let tag = prefix >> 4;

            match item_type {
                1 => { // Global items
                    match tag {
                        0 => global.usage_page = raw as u16,
                        1 => global.logical_min = sign_extend(raw, size),
                        2 => {
                            global.logical_max = if global.logical_min < 0 {
                                sign_extend(raw, size)
                            } else { raw as i32 };
                        }
                        7 => global.report_size = raw,
                        8 => {
                            if raw == 0 || raw > 255 { return Err(HidReportError::MalformedDescriptor); }
                            global.report_id = raw as u8;
                        }
                        9 => global.report_count = raw,
                        10 => {
                            if global_depth == MAX_GLOBAL_DEPTH { return Err(HidReportError::CollectionOverflow); }
                            global_stack[global_depth] = global;
                            global_depth += 1;
                        }
                        11 => {
                            if global_depth == 0 { return Err(HidReportError::MalformedDescriptor); }
                            global_depth -= 1;
                            global = global_stack[global_depth];
                        }
                        _ => {}
                    }
                }
                2 => { // Local items
                    let usage = raw as u16;
                    match tag {
                        0 => {
                            if local.first_usage.is_none() { local.first_usage = Some(usage); }
                            if (local.usage_count as usize) < local.usages.len() {
                                local.usages[local.usage_count as usize] = usage;
                                local.usage_count += 1;
                            }
                        },
                        1 => local.usage_min = Some(usage),
                        2 => local.usage_max = Some(usage),
                        _ => {}
                    }
                }
                0 => { // Main items
                    match tag {
                        8 => {
                            if global.report_size == 0 || global.report_size > 32
                                || global.report_count == 0 || global.report_count > 128 {
                                return Err(HidReportError::InvalidField);
                            }
                            let count = global.report_count as usize;
                            let slots_end = map.value_count as usize + count;
                            if map.field_count as usize == MAX_HID_INPUT_FIELDS {
                                return Err(HidReportError::TooManyFields);
                            }
                            if slots_end > MAX_HID_VALUES { return Err(HidReportError::TooManyValues); }
                            let bits = global.report_size.checked_mul(global.report_count)
                                .ok_or(HidReportError::ArithmeticOverflow)?;
                            let report_offset = map.report_bits[global.report_id as usize];
                            let next_offset = report_offset.checked_add(bits)
                                .ok_or(HidReportError::ArithmeticOverflow)?;
                            let usage_min = local.usage_min.or(local.first_usage).unwrap_or(0);
                            let usage_max = local.usage_max.unwrap_or(usage_min);
                            map.fields[map.field_count as usize] = Some(HidInputField {
                                report_id: global.report_id,
                                bit_offset: report_offset,
                                report_size: global.report_size as u8,
                                report_count: global.report_count as u8,
                                value_slot: map.value_count,
                                usage_page: global.usage_page,
                                usage_min,
                                usage_max,
                                usage_list: local.usages,
                                usage_count: local.usage_count,
                                logical_min: global.logical_min,
                                logical_max: global.logical_max,
                                flags: raw as u8,
                                application_page: application.page,
                                application_usage: application.usage,
                            });
                            map.field_count += 1;
                            map.value_count = slots_end as u16;
                            map.report_bits[global.report_id as usize] = next_offset;
                        }
                        10 => { // Collection
                            if collection_depth == MAX_COLLECTION_DEPTH {
                                return Err(HidReportError::CollectionOverflow);
                            }
                            let usage = local.first_usage.or(local.usage_min).unwrap_or(0);
                            collections[collection_depth] = application;
                            collection_depth += 1;
                            if raw as u8 == 1 {
                                application = Collection { page: global.usage_page, usage };
                            }
                        }
                        12 => {
                            if collection_depth == 0 { return Err(HidReportError::MalformedDescriptor); }
                            collection_depth -= 1;
                            application = collections[collection_depth];
                        }
                        _ => {}
                    }
                    local.clear();
                }
                _ => {}
            }
        }
        if collection_depth != 0 || global_depth != 0 || map.field_count == 0 {
            return Err(HidReportError::MalformedDescriptor);
        }
        Ok(map)
    }

    pub const fn field_count(&self) -> usize { self.field_count as usize }
    pub const fn value_count(&self) -> usize { self.value_count as usize }
    pub fn field(&self, index: usize) -> Option<HidInputField> {
        if index >= self.field_count as usize { None } else { self.fields[index] }
    }
    pub const fn report_bytes(&self, report_id: u8) -> usize {
        ((self.report_bits[report_id as usize] as usize + 7) / 8)
            + if report_id == 0 { 0 } else { 1 }
    }
}

const fn sign_extend(raw: u32, size: usize) -> i32 {
    if size == 0 { return 0; }
    if size >= 4 { return raw as i32; }
    let shift = 32 - size * 8;
    ((raw << shift) as i32) >> shift
}

fn field_usage(field: HidInputField, element: usize) -> u16 {
    if element < field.usage_count as usize {
        field.usage_list[element]
    } else if field.usage_max > field.usage_min {
        field.usage_min.saturating_add(element as u16).min(field.usage_max)
    } else {
        field.usage_min
    }
}
fn extract_bits(bytes: &[u8], bit_offset: u32, bit_size: u8) -> Option<u32> {
    if bit_size == 0 || bit_size > 32 { return None; }
    let mut value = 0u32;
    let mut bit = 0u32;
    while bit < bit_size as u32 {
        let absolute = bit_offset.checked_add(bit)?;
        let byte_index = (absolute / 8) as usize;
        if byte_index >= bytes.len() { return None; }
        let bit_index = absolute & 7;
        if bytes[byte_index] & (1 << bit_index) != 0 { value |= 1 << bit; }
        bit += 1;
    }
    Some(value)
}
fn decode_signed(raw: u32, size: u8, logical_min: i32) -> i32 {
    if logical_min < 0 && size < 32 {
        let shift = 32 - size as u32;
        ((raw << shift) as i32) >> shift
    } else { raw as i32 }
}
fn normalize_absolute(value: i32, min: i32, max: i32) -> u16 {
    if max <= min { return value.clamp(0, u16::MAX as i32) as u16; }
    let clamped = value.clamp(min, max) as i64;
    let span = (max as i64) - (min as i64);
    (((clamped - min as i64) * u16::MAX as i64) / span)
        .clamp(0, u16::MAX as i64) as u16
}
fn normalize_axis(value: i32, min: i32, max: i32) -> i16 {
    if max <= min { return value.clamp(i16::MIN as i32, i16::MAX as i32) as i16; }
    let clamped = value.clamp(min, max) as i64;
    let span = (max as i64) - (min as i64);
    let scaled = ((clamped - min as i64) * 65535 / span) - 32768;
    scaled.clamp(i16::MIN as i64, i16::MAX as i64) as i16
}

/// Stateful decoder for one HID interface/collection. Instantiate one per
/// InputDeviceId; report state is never shared across composite interfaces.
pub struct HidReportDecoder {
    device: InputDeviceId,
    previous: [i32; MAX_HID_VALUES],
    initialized: [bool; MAX_HID_VALUES],
    contact_active: bool,
    resync_required: bool,
    pub dropped_events: u64,
}
impl HidReportDecoder {
    pub const fn new(device: InputDeviceId) -> Self {
        Self {
            device, previous: [0; MAX_HID_VALUES], initialized: [false; MAX_HID_VALUES],
            contact_active: false, resync_required: false, dropped_events: 0,
        }
    }
    pub const fn device_id(&self) -> InputDeviceId { self.device }
    pub fn reset(&mut self) {
        self.previous = [0; MAX_HID_VALUES];
        self.initialized = [false; MAX_HID_VALUES];
        self.contact_active = false;
        self.resync_required = false;
    }

    /// Decodes one successful interrupt-IN report. For report-ID descriptors,
    /// the first byte is validated and excluded from bit offsets.
    pub fn publish_report<const N: usize>(
        &mut self,
        map: &HidReportMap,
        bytes: &[u8],
        producer: &mut SpscProducer<'_, InputEvent, N>,
    ) -> Result<HidPublishStats, HidReportError> {
        if bytes.is_empty() { return Err(HidReportError::ReportTooShort); }
        let has_ids = map.fields[..map.field_count as usize].iter().flatten()
            .any(|field| field.report_id != 0);
        let report_id = if has_ids { bytes[0] } else { 0 };
        if has_ids && map.fields[..map.field_count as usize].iter().flatten()
            .all(|field| field.report_id != report_id) {
            return Err(HidReportError::ReportIdMismatch);
        }
        let payload = if has_ids { &bytes[1..] } else { bytes };
        let expected = map.report_bytes(report_id).saturating_sub(if has_ids { 1 } else { 0 });
        if payload.len() < expected { return Err(HidReportError::ReportTooShort); }

        let mut stats = HidPublishStats::default();
        if self.resync_required {
            // Queue overflow can lose an edge. Reset the consumer's per-device
            // state before replaying the next full report snapshot.
            if producer.push(InputEvent::DeviceStateReset { device: self.device }).is_err() {
                stats.dropped = 1;
                self.dropped_events = self.dropped_events.saturating_add(1);
                return Ok(stats);
            }
            stats.published = stats.published.saturating_add(1);
            self.previous = [0; MAX_HID_VALUES];
            self.initialized = [false; MAX_HID_VALUES];
            self.contact_active = false;
            self.resync_required = false;
        }
        let mut modifiers = 0u8;
        for field in map.fields[..map.field_count as usize].iter().flatten() {
            if field.report_id != report_id || field.usage_page != 0x07 || !field.is_variable() { continue; }
            for element in 0..field.report_count as usize {
                let usage = field.usage_min.saturating_add(element as u16);
                if (0xE0..=0xE7).contains(&usage) {
                    let bit = field.bit_offset + element as u32 * field.report_size as u32;
                    let raw = extract_bits(payload, bit, field.report_size).ok_or(HidReportError::ReportTooShort)?;
                    if raw != 0 { modifiers |= 1 << (usage - 0xE0); }
                }
            }
        }
        let mut pointer_x = 0i32;
        let mut pointer_y = 0i32;
        let mut pointer_changed = false;
        let mut pointer_buttons = 0u32;
        let mut pointer_button_seen = false;
        let mut pointer_wheel_x = 0i16;
        let mut pointer_wheel_y = 0i16;

        for field_index in 0..map.field_count as usize {
            let field = match map.fields[field_index] { Some(f) if f.report_id == report_id => f, _ => continue };
            if field.is_constant() { continue; }
            for element in 0..field.report_count as usize {
                let bit_offset = field.bit_offset
                    .checked_add((element as u32).saturating_mul(field.report_size as u32))
                    .ok_or(HidReportError::ArithmeticOverflow)?;
                let raw = extract_bits(payload, bit_offset, field.report_size)
                    .ok_or(HidReportError::ReportTooShort)?;
                let value = decode_signed(raw, field.report_size, field.logical_min);
                let slot = field.value_slot as usize + element;
                if slot >= MAX_HID_VALUES { return Err(HidReportError::InvalidField); }
                let usage = field_usage(field, element);
                let old = self.previous[slot];
                let had_old = self.initialized[slot];
                let variable = field.is_variable();

                if field.usage_page == 0x07 {
                    if variable {
                        if usage != 0 && ((!had_old && value != 0) || (had_old && old != value)) {
                            self.emit(InputEvent::HidKey {
                                device: self.device, usage_page: 0x07, usage,
                                pressed: value != 0, modifiers,
                            }, producer, &mut stats);
                        }
                    } else {
                        if had_old && old > 0 && old != value {
                            self.emit(InputEvent::HidKey { device: self.device, usage_page: 0x07,
                                usage: old as u16, pressed: false, modifiers }, producer, &mut stats);
                        }
                        if value > 0 && (!had_old || old != value) {
                            self.emit(InputEvent::HidKey { device: self.device, usage_page: 0x07,
                                usage: value as u16, pressed: true, modifiers }, producer, &mut stats);
                        }
                    }
                } else if field.usage_page == 0x09 {
                    let gamepad = field.application_page == 0x01
                        && (field.application_usage == 0x04 || field.application_usage == 0x05);
                    if gamepad {
                        if variable {
                            if usage != 0 && ((!had_old && value != 0) || (had_old && old != value)) {
                                self.emit(InputEvent::GamepadButton {
                                    device: self.device, button: usage, pressed: value != 0,
                                }, producer, &mut stats);
                            }
                        } else {
                            if had_old && old > 0 && old != value {
                                self.emit(InputEvent::GamepadButton {
                                    device: self.device, button: old as u16, pressed: false,
                                }, producer, &mut stats);
                            }
                            if value > 0 && (!had_old || old != value) {
                                self.emit(InputEvent::GamepadButton {
                                    device: self.device, button: value as u16, pressed: true,
                                }, producer, &mut stats);
                            }
                        }
                    } else if field.application_page == 0x01
                        && (field.application_usage == 0x01 || field.application_usage == 0x02) {
                        pointer_button_seen = true;
                    }
                } else if field.usage_page == 0x0C {
                    if usage == 0x0238 {
                        pointer_wheel_x = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                        pointer_changed = true;
                    } else if variable {
                        if usage != 0 && ((!had_old && value != 0) || (had_old && old != value)) {
                            self.emit(InputEvent::ConsumerControl { device: self.device,
                                usage_page: 0x0C, usage, pressed: value != 0 }, producer, &mut stats);
                        }
                    } else {
                        if had_old && old > 0 && old != value {
                            self.emit(InputEvent::ConsumerControl { device: self.device,
                                usage_page: 0x0C, usage: old as u16, pressed: false }, producer, &mut stats);
                        }
                        if value > 0 && (!had_old || old != value) {
                            self.emit(InputEvent::ConsumerControl { device: self.device,
                                usage_page: 0x0C, usage: value as u16, pressed: true }, producer, &mut stats);
                        }
                    }
                } else if field.usage_page == 0x01 {
                    match usage {
                        0x30 | 0x31 if field.is_relative() => {
                            if usage == 0x30 { pointer_x = value; } else { pointer_y = value; }
                            pointer_changed = true;
                        }
                        0x38 if field.is_relative() => {
                            pointer_wheel_y = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
                            pointer_changed = true;
                        }
                        0x30..=0x38 if field.application_usage == 0x04
                            || field.application_usage == 0x05 => {
                            let axis = match usage {
                                0x30 => GamepadAxis::LeftX, 0x31 => GamepadAxis::LeftY,
                                0x32 => GamepadAxis::LeftTrigger, 0x33 => GamepadAxis::RightX,
                                0x34 => GamepadAxis::RightY, 0x35 => GamepadAxis::RightTrigger,
                                0x36 => GamepadAxis::Other(6), 0x37 => GamepadAxis::Other(7),
                                _ => GamepadAxis::Other(8),
                            };
                            if !had_old || old != value {
                                self.emit(InputEvent::GamepadAxis {
                                    device: self.device, axis,
                                    value: normalize_axis(value, field.logical_min, field.logical_max),
                                }, producer, &mut stats);
                            }
                        }
                        _ => {}
                    }
                }
                self.previous[slot] = value;
                self.initialized[slot] = true;
            }
        }
        if pointer_button_seen {
            for field in map.fields[..map.field_count as usize].iter().flatten() {
                if field.report_id != report_id || field.usage_page != 0x09
                    || (field.application_page == 0x01
                        && (field.application_usage == 0x04 || field.application_usage == 0x05)) { continue; }
                for element in 0..field.report_count as usize {
                    let slot = field.value_slot as usize + element;
                    let button = if field.is_variable() { field_usage(*field, element) }
                        else { self.previous[slot].max(0) as u16 };
                    if self.previous[slot] != 0 && (1..=32).contains(&button) {
                        pointer_buttons |= 1u32 << (button - 1);
                    }
                }
            }
        }
        // Digitizer collections use Generic Desktop X/Y plus Digitizer-page
        // usages for tip, pressure, range and contact identity. Decode them
        // separately from relative mouse axes.
        let mut touch_x = 0u16;
        let mut touch_y = 0u16;
        let mut touch_pressure = 0u16;
        let mut touch_contact = 0u16;
        let mut touch_tip = false;
        let mut touch_seen = false;
        let mut pen_x = 0u16;
        let mut pen_y = 0u16;
        let mut pen_pressure = 0u16;
        let mut pen_tilt_x = 0i16;
        let mut pen_tilt_y = 0i16;
        let mut pen_tip = false;
        let mut pen_in_range = false;
        let mut pen_buttons = 0u32;
        let mut pen_seen = false;
        for field in map.fields[..map.field_count as usize].iter().flatten() {
            if field.report_id != report_id { continue; }
            let is_touch = field.application_page == 0x0D && field.application_usage == 0x04;
            let is_pen = field.application_page == 0x0D && field.application_usage == 0x02;
            if !is_touch && !is_pen { continue; }
            for element in 0..field.report_count as usize {
                let usage = field_usage(*field, element);
                let bit = field.bit_offset + element as u32 * field.report_size as u32;
                let raw = extract_bits(payload, bit, field.report_size).ok_or(HidReportError::ReportTooShort)?;
                let value = decode_signed(raw, field.report_size, field.logical_min);
                if is_touch {
                    touch_seen = true;
                    match (field.usage_page, usage) {
                        (0x01, 0x30) => touch_x = normalize_absolute(value, field.logical_min, field.logical_max),
                        (0x01, 0x31) => touch_y = normalize_absolute(value, field.logical_min, field.logical_max),
                        (0x0D, 0x30) => touch_pressure = normalize_absolute(value, field.logical_min, field.logical_max),
                        (0x0D, 0x42) => touch_tip = value != 0,
                        (0x0D, 0x51) => touch_contact = value.max(0) as u16,
                        _ => {}
                    }
                } else {
                    pen_seen = true;
                    match (field.usage_page, usage) {
                        (0x01, 0x30) => pen_x = normalize_absolute(value, field.logical_min, field.logical_max),
                        (0x01, 0x31) => pen_y = normalize_absolute(value, field.logical_min, field.logical_max),
                        (0x0D, 0x30) => pen_pressure = normalize_absolute(value, field.logical_min, field.logical_max),
                        (0x0D, 0x32) => pen_in_range = value != 0,
                        (0x0D, 0x42) => pen_tip = value != 0,
                        (0x0D, 0x3D) => pen_tilt_x = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                        (0x0D, 0x3E) => pen_tilt_y = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
                        (0x09, button @ 1..=32) if value != 0 => pen_buttons |= 1u32 << (button - 1),
                        _ => {}
                    }
                }
            }
        }
        if touch_seen {
            let phase = if touch_tip && !self.contact_active {
                Some(super::event::ContactPhase::Down)
            } else if touch_tip {
                Some(super::event::ContactPhase::Move)
            } else if self.contact_active {
                Some(super::event::ContactPhase::Up)
            } else { None };
            if let Some(phase) = phase {
                self.emit(InputEvent::TouchContact {
                    device: self.device, contact: touch_contact, phase,
                    x: touch_x, y: touch_y, pressure: touch_pressure,
                }, producer, &mut stats);
            }
            self.contact_active = touch_tip;
        }
        if pen_seen {
            self.emit(InputEvent::Tablet {
                device: self.device, x: pen_x, y: pen_y, pressure: pen_pressure,
                tilt_x: pen_tilt_x, tilt_y: pen_tilt_y, buttons: pen_buttons,
                in_range: pen_in_range, tip: pen_tip,
            }, producer, &mut stats);
        }
        if pointer_changed || pointer_button_seen {
            self.emit(InputEvent::PointerMotion {
                device: self.device, dx: pointer_x, dy: pointer_y,
                wheel_x: pointer_wheel_x, wheel_y: pointer_wheel_y, buttons: pointer_buttons,
            }, producer, &mut stats);
        }
        Ok(stats)
    }

    fn emit<const N: usize>(
        &mut self, event: InputEvent, producer: &mut SpscProducer<'_, InputEvent, N>,
        stats: &mut HidPublishStats,
    ) {
        if producer.push(event).is_ok() {
            stats.published = stats.published.saturating_add(1);
        } else {
            stats.dropped = stats.dropped.saturating_add(1);
            self.dropped_events = self.dropped_events.saturating_add(1);
            self.resync_required = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::queue::SpscQueue;

    // Keyboard application, 8 one-bit modifier fields, 6 key-array bytes.
    const KEYBOARD_DESCRIPTOR: &[u8] = &[
        0x05,0x01, 0x09,0x06, 0xA1,0x01,
        0x05,0x07, 0x19,0xE0, 0x29,0xE7, 0x15,0x00, 0x25,0x01,
        0x75,0x01, 0x95,0x08, 0x81,0x02,
        0x95,0x01, 0x75,0x08, 0x81,0x01,
        0x95,0x06, 0x75,0x08, 0x15,0x00, 0x25,0x65,
        0x05,0x07, 0x19,0x00, 0x29,0x65, 0x81,0x00, 0xC0,
    ];

    #[test]
    fn parses_keyboard_fields_and_report_size() {
        let map = HidReportMap::parse(KEYBOARD_DESCRIPTOR).unwrap();
        assert!(map.field_count() >= 2);
        assert_eq!(map.report_bytes(0), 8);
    }

    #[test]
    fn decodes_generic_keyboard_array_edges() {
        let map = HidReportMap::parse(KEYBOARD_DESCRIPTOR).unwrap();
        let mut q = SpscQueue::<InputEvent, 16>::new();
        let (mut producer, mut consumer) = q.split();
        let mut decoder = HidReportDecoder::new(InputDeviceId(17));
        decoder.publish_report(&map, &[0,0,4,0,0,0,0,0], &mut producer).unwrap();
        assert!(matches!(consumer.pop(), Some(InputEvent::HidKey {
            usage_page: 7, usage: 4, pressed: true, ..
        })));
        decoder.publish_report(&map, &[0,0,0,0,0,0,0,0], &mut producer).unwrap();
        assert!(matches!(consumer.pop(), Some(InputEvent::HidKey {
            usage_page: 7, usage: 4, pressed: false, ..
        })));
    }

    #[test]
    fn decodes_touchscreen_contact_lifecycle_and_coordinates() {
        // Application Touch Screen: absolute X/Y, Tip Switch, padding, Contact ID.
        const TOUCH: &[u8] = &[
            0x05,0x0D, 0x09,0x04, 0xA1,0x01,
            0x05,0x01, 0x09,0x30, 0x09,0x31,
            0x15,0x00, 0x26,0xE8,0x03, 0x75,0x10, 0x95,0x02, 0x81,0x02,
            0x05,0x0D, 0x09,0x42, 0x15,0x00, 0x25,0x01,
            0x75,0x01, 0x95,0x01, 0x81,0x02,
            0x75,0x07, 0x95,0x01, 0x81,0x01,
            0x09,0x51, 0x75,0x08, 0x95,0x01, 0x81,0x02,
            0xC0,
        ];
        let map = HidReportMap::parse(TOUCH).unwrap();
        assert_eq!(map.report_bytes(0), 6);
        let mut q = SpscQueue::<InputEvent, 8>::new();
        let (mut producer, mut consumer) = q.split();
        let mut decoder = HidReportDecoder::new(InputDeviceId(22));
        decoder.publish_report(&map, &[0xF4,0x01,0xEE,0x02,0x01,9], &mut producer).unwrap();
        assert!(matches!(consumer.pop(), Some(InputEvent::TouchContact {
            contact: 9, phase: super::super::event::ContactPhase::Down,
            x, y, ..
        }) if x > 30_000 && y > 45_000));
        decoder.publish_report(&map, &[0xF4,0x01,0xEE,0x02,0x00,9], &mut producer).unwrap();
        assert!(matches!(consumer.pop(), Some(InputEvent::TouchContact {
            contact: 9, phase: super::super::event::ContactPhase::Up, ..
        })));
    }

    #[test]
    fn decodes_gamepad_axes_and_button_edges() {
        const GAMEPAD: &[u8] = &[
            0x05,0x01, 0x09,0x05, 0xA1,0x01,
            0x09,0x30, 0x09,0x31, 0x15,0x81, 0x25,0x7F,
            0x75,0x08, 0x95,0x02, 0x81,0x02,
            0x05,0x09, 0x19,0x01, 0x29,0x04, 0x15,0x00, 0x25,0x01,
            0x75,0x01, 0x95,0x04, 0x81,0x02,
            0x75,0x04, 0x95,0x01, 0x81,0x01,
            0xC0,
        ];
        let map = HidReportMap::parse(GAMEPAD).unwrap();
        assert_eq!(map.report_bytes(0), 3);
        let mut q = SpscQueue::<InputEvent, 16>::new();
        let (mut producer, mut consumer) = q.split();
        let mut decoder = HidReportDecoder::new(InputDeviceId(31));
        decoder.publish_report(&map, &[0x80,0x7F,0x05], &mut producer).unwrap();
        assert!(matches!(consumer.pop(), Some(InputEvent::GamepadAxis {
            axis: GamepadAxis::LeftX, value: i16::MIN, ..
        })));
        assert!(matches!(consumer.pop(), Some(InputEvent::GamepadAxis {
            axis: GamepadAxis::LeftY, value: i16::MAX, ..
        })));
        assert!(matches!(consumer.pop(), Some(InputEvent::GamepadButton { button: 1, pressed: true, .. })));
        assert!(matches!(consumer.pop(), Some(InputEvent::GamepadButton { button: 3, pressed: true, .. })));
        decoder.publish_report(&map, &[0x80,0x7F,0x00], &mut producer).unwrap();
        assert!(matches!(consumer.pop(), Some(InputEvent::GamepadButton { button: 1, pressed: false, .. })));
        assert!(matches!(consumer.pop(), Some(InputEvent::GamepadButton { button: 3, pressed: false, .. })));
    }

    #[test]
    fn queue_overflow_resets_consumer_before_replaying_state() {
        let map = HidReportMap::parse(KEYBOARD_DESCRIPTOR).unwrap();
        let mut q = SpscQueue::<InputEvent, 4>::new();
        let (mut producer, mut consumer) = q.split();
        for id in 1..=3 {
            producer.push(InputEvent::DeviceConnected { device: InputDeviceId(id) }).unwrap();
        }
        let mut decoder = HidReportDecoder::new(InputDeviceId(40));
        let stats = decoder.publish_report(&map, &[0,0,4,0,0,0,0,0], &mut producer).unwrap();
        assert_eq!(stats.dropped, 1);
        while consumer.pop().is_some() {}
        decoder.publish_report(&map, &[0,0,4,0,0,0,0,0], &mut producer).unwrap();
        assert!(matches!(consumer.pop(), Some(InputEvent::DeviceStateReset { device: InputDeviceId(40) })));
        assert!(matches!(consumer.pop(), Some(InputEvent::HidKey { usage: 4, pressed: true, .. })));
    }

    #[test]
    fn rejects_truncated_and_wrong_id_reports() {
        assert_eq!(HidReportMap::parse(&[0x75]), Err(HidReportError::TruncatedDescriptor));
        let descriptor = [0x85,1,0x75,8,0x95,1,0x81,2];
        let map = HidReportMap::parse(&descriptor).unwrap();
        let mut q = SpscQueue::<InputEvent, 4>::new();
        let (mut producer, _) = q.split();
        let mut decoder = HidReportDecoder::new(InputDeviceId(1));
        assert_eq!(decoder.publish_report(&map, &[2,0], &mut producer), Err(HidReportError::ReportIdMismatch));
    }
}
