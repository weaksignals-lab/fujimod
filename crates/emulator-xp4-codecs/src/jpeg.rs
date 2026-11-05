//! Register-driven JPEG transport with a caller-declared encoding law.
//!
//! The selected law uses Y0/Y1/Cb/Cr pairs, big-endian uploaded zigzag
//! quantizers, SOI..EOI output, source sampling at START, and a deterministic
//! completion delay. These numerical, framing and timing choices are explicit
//! model inputs; register transport alone does not establish silicon fidelity.
use fujimod_emulator_devices::{Cx, Device};
use fujimod_emulator_image_codecs::jpeg::{Codec, Packed422Layout};

pub const BASE: u64 = 0xfffa0000;
pub const WINDOW: u64 = 0x134;
pub const ADDRESS_HIGH: u64 = 0xfff4a740;
const READY: u32 = 1 << 16;
const DONE: u32 = 1 << 4;

/// Explicit admission and numerical law for the supported encoder subset.
/// Raw control values are matched without assigning them hardware semantics.
#[derive(Clone, Copy, Debug)]
pub struct Packed422EncodingLaw {
    pub raw_control_48: u32,
    pub raw_control_80: u16,
    pub restart_mcus: u16,
    pub completion_ticks: u64,
}
struct Pending {
    due: u64,
    source: Vec<u8>,
    destination: u64,
    capacity: usize,
    layout: Packed422Layout,
    codec: Codec,
}
pub struct Encoder {
    law: Option<Packed422EncodingLaw>,
    registers: [u32; 0x134 / 4],
    high: [u32; 2],
    tables: Vec<u8>,
    uploading: bool,
    ready: bool,
    status: u32,
    count: u32,
    pending: Option<Pending>,
}
impl Encoder {
    pub fn new(law: Option<Packed422EncodingLaw>) -> Result<Self, String> {
        if law.is_some_and(|v| v.completion_ticks == 0) {
            return Err("JPEG completion delay must be positive".into());
        }
        Ok(Self { law, registers: [0; 0x134 / 4], high: [0; 2],
            tables: Vec::new(), uploading: false, ready: false,
            status: 0, count: 0, pending: None })
    }
    fn offset(address: u64, size: u8) -> Option<usize> {
        if size != 4 || address & 3 != 0 { return None; }
        address.checked_sub(BASE).filter(|o| *o < WINDOW).map(|o| o as usize)
    }
    fn start(&mut self, cx: &mut Cx<'_>) -> Result<(), String> {
        let law = self.law.ok_or("JPEG START requires an explicit encoding law")?;
        let r = &self.registers;
        if !self.ready || self.uploading || self.tables.len() != 128 || self.pending.is_some()
            || r[0x48/4] != law.raw_control_48 || r[0x80/4] != u32::from(law.raw_control_80)
            || r[0x12c/4] != 0 || r[0x0c/4] != 0x000a0003
            || r[0x28/4] != 0x00fd0000 || r[0x40/4] != 1
            || r[0x11c/4] != 1 || r[0x120/4] != 0 {
            return Err("JPEG configuration exceeds selected encoding law".into());
        }
        let pitch = r[0x130/4];
        if pitch > 65535 || pitch & 7 != 0 { return Err("invalid JPEG pitch".into()); }
        let layout = Packed422Layout { width: (r[0x44/4] & 65535) as usize,
            height: (r[0x44/4] >> 16) as usize, stride_bytes: pitch as usize * 2 };
        let span = layout.stride_bytes.checked_mul(layout.height).ok_or("JPEG source span overflow")?;
        let capacity = r[0x110/4] as usize;
        if capacity < 8 || capacity & 7 != 0 { return Err("invalid JPEG capacity".into()); }
        let mut codec = Codec::default();
        codec.tables(&self.tables)?;
        codec.validate(span, layout, law.restart_mcus, capacity - 2)?;
        let source = (u64::from(self.high[0]) << 32) | u64::from(r[0x100/4]);
        let destination = (u64::from(self.high[1]) << 32) | u64::from(r[0x10c/4]);
        let source_end = source.checked_add(span as u64).ok_or("JPEG source end overflow")?;
        let destination_end = destination.checked_add(capacity as u64).ok_or("JPEG destination end overflow")?;
        if source & 15 != 0 || destination & 15 != 0 || source_end > 1<<34
            || destination_end > 1<<34 || source < destination_end && destination < source_end {
            return Err("JPEG DMA spans overlap or escape aligned 34-bit memory".into());
        }
        let due = cx.now().checked_add(law.completion_ticks).ok_or("JPEG deadline overflow")?;
        let source = cx.ram_bytes(source, span)?;
        self.pending = Some(Pending { due, source, destination, capacity, layout, codec });
        self.count = 0;
        self.status = 0;
        cx.set_irq(135, false);
        Ok(())
    }
}
impl Device for Encoder {
    fn name(&self) -> &'static str { "xp4-jpeg-encoder" }
    fn read(&mut self, cx: &mut Cx<'_>, address: u64, size: u8) -> u64 {
        if size == 4 && [ADDRESS_HIGH, ADDRESS_HIGH+4].contains(&address) {
            return self.high[((address-ADDRESS_HIGH)/4) as usize].into();
        }
        let Some(o) = Self::offset(address, size) else { cx.unmodeled(); return 0; };
        match o {
            0 => 0,
            0x24 => u64::from(self.status | if self.ready && self.pending.is_none() { READY } else { 0 }),
            0x94 => self.count.into(),
            0x0c|0x28|0x40|0x44|0x48|0x80|0x84|0x100|0x10c|0x110|0x118|0x11c|0x120|0x12c|0x130 => self.registers[o/4].into(),
            _ => { cx.unmodeled(); 0 }
        }
    }
    fn write(&mut self, cx: &mut Cx<'_>, address: u64, size: u8, value: u64) {
        let Ok(value) = u32::try_from(value) else { return cx.unmodeled(); };
        if size == 4 && [ADDRESS_HIGH, ADDRESS_HIGH+4].contains(&address) {
            if value > 3 || self.pending.is_some() { return cx.fail("invalid or busy JPEG high address"); }
            self.high[((address-ADDRESS_HIGH)/4) as usize] = value; return;
        }
        let Some(o) = Self::offset(address, size) else { return cx.unmodeled(); };
        if o == 0 && value == 0x100 {
            self.pending = None; self.status = 0; self.count = 0;
            self.registers.fill(0); self.high.fill(0); self.tables.clear(); self.uploading = false;
            cx.set_irq(135, false); return;
        }
        if o == 0x24 {
            if value & !(READY | DONE) != 0 { return cx.unsupported("unsupported JPEG acknowledgement"); }
            self.status &= !(value & DONE);
            cx.set_irq(135, self.status & DONE != 0); return;
        }
        if self.pending.is_some() { return cx.fail("JPEG register write while busy"); }
        match o {
            0 if value == 1 => { if let Err(e) = self.start(cx) { cx.fail(e); } return; }
            0 if value == 0 => return,
            0x118 if value == 1 => self.ready = true,
            0x84 if value == 0x100 => { self.tables.clear(); self.uploading = true; }
            0x84 if value == 1 && self.uploading && self.tables.len() == 128 => self.uploading = false,
            0x8c if self.uploading && self.tables.len() < 128 => { self.tables.extend_from_slice(&value.to_be_bytes()); return; }
            0x0c|0x28|0x40|0x44|0x48|0x80|0x100|0x10c|0x110|0x11c|0x120|0x12c|0x130 => {},
            _ => return cx.unsupported("unsupported JPEG register or command"),
        }
        self.registers[o/4] = value;
    }
    fn next_deadline(&self, _: u64) -> Option<u64> { self.pending.as_ref().map(|p| p.due) }
    fn advance(&mut self, cx: &mut Cx<'_>) {
        if !self.pending.as_ref().is_some_and(|p| p.due <= cx.now()) { return; }
        let p = self.pending.take().unwrap();
        let mut output = Vec::new();
        let law = self.law.unwrap();
        if let Err(e) = p.codec.encode(&p.source, p.layout, law.restart_mcus, &mut output, p.capacity-2) {
            return cx.fail(format!("JPEG encoding failed: {e}"));
        }
        output.splice(0..0, [0xff, 0xd8]);
        if let Err(e) = cx.ram_write(p.destination, &output) { return cx.fail(e); }
        self.count = output.len() as u32;
        self.status = DONE;
        cx.set_irq(135, true);
    }
}

#[cfg(test)]
mod tests;
