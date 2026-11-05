use super::*;
use fujimod_emulator_devices::{Host, Record, test_host::recording::RecordingHost};
use fujimod_emulator_image_codecs::jpeg::decode;
struct Tracking { inner: RecordingHost, effects: Vec<&'static str> }
impl Host for Tracking {
    fn now(&self) -> u64 { self.inner.now }
    fn schedule(&mut self, d: usize, at: u64, t: u64) { self.inner.schedule(d, at, t); }
    fn set_irq(&mut self, n: u32, high: bool) {
        if high { self.effects.push("irq"); }
        self.inner.set_irq(n, high);
    }
    fn raise_irq(&mut self, c: usize, n: u32) -> Result<(), String> { self.inner.raise_irq(c,n) }
    fn ram_read(&self, a: u64, b: &mut [u8]) -> Result<(), String> { self.inner.ram_read(a,b) }
    fn ram_write(&mut self, a: u64, b: &[u8]) -> Result<(), String> {
        self.inner.ram_write(a,b)?; self.effects.push("dma"); Ok(())
    }
    fn record(&mut self, _: Record) {}
}
fn write(d: &mut Encoder, h: &mut Tracking, a: u64, v: u32) -> Result<(), String> {
    let mut cx=Cx::new(h, 0, 0, 0); d.write(&mut cx,a,4,v.into());
    cx.take_refusal().map_or(Ok(()),Err)
}
fn read(d: &mut Encoder, h: &mut Tracking, o: u64) -> u32 {
    let mut cx=Cx::new(h, 0, 0, 0); let v=d.read(&mut cx,BASE+o,4);
    assert!(cx.refusal().is_none()); v as u32
}
fn advance(d: &mut Encoder, h: &mut Tracking) -> Result<(),String> {
    let mut cx=Cx::new(h, 0, 0, 0); d.advance(&mut cx);
    cx.take_refusal().map_or(Ok(()),Err)
}
fn fixture(value: u8) -> (Encoder,Tracking) {
    let mut d=Encoder::new(Some(Packed422EncodingLaw { raw_control_48:0,
        raw_control_80:4, restart_mcus:3, completion_ticks:10 })).unwrap();
    let mut h=Tracking { inner:RecordingHost { ram:vec![0xa5;65536],ram_base:1<<32,
        ..Default::default() },effects:Vec::new() };
    for y in 0..24 { for x in (0..80).step_by(2) {
        let i=y*176+x*2;
        h.inner.ram[i..i+4].copy_from_slice(&[value+(x/8) as u8,value+8,80,160]);
    }}
    for (o,v) in [(0x118,1),(0x11c,1),(0x120,0),(0x0c,0xa0003),(0x28,0xfd0000),(0x40,1),
        (0x44,(24<<16)|80),(0x48,0),(0x80,4),(0x130,88),
        (0x100,0),(0x10c,0x4000),(0x110,8192),(0x84,0x100)] {
        write(&mut d,&mut h,BASE+o,v).unwrap();
    }
    for _ in 0..32 { write(&mut d,&mut h,BASE+0x8c,0x03030303).unwrap(); }
    write(&mut d,&mut h,BASE+0x84,1).unwrap();
    write(&mut d,&mut h,ADDRESS_HIGH,1).unwrap();
    write(&mut d,&mut h,ADDRESS_HIGH+4,1).unwrap();
    (d,h)
}
#[test]
fn padded_high_address_pixels_complete_in_order() {
    let mut streams=Vec::new();
    for value in [32,144] {
        let (mut d,mut h)=fixture(value);let before=h.inner.ram.clone();
        assert_eq!(read(&mut d,&mut h,0x24),READY);
        write(&mut d,&mut h,BASE,1).unwrap();
        assert_eq!(d.next_deadline(0),Some(10));
        assert_eq!(read(&mut d,&mut h,0x94),0);
        h.inner.now=9;advance(&mut d,&mut h).unwrap();
        assert_eq!(h.inner.ram,before);assert!(h.effects.is_empty());
        // Source sampling occurs at START, not completion.
        h.inner.ram[..176*24].fill(0);
        h.inner.now=10;advance(&mut d,&mut h).unwrap();
        assert_eq!(h.effects,vec!["dma","irq"]);
        assert_eq!(h.inner.irqs.last(),Some(&(135,true)));
        let n=read(&mut d,&mut h,0x94) as usize;
        let bytes=h.inner.ram[0x4000..0x4000+n].to_vec();
        let (pixels,w,ht)=decode(&bytes,80,24).unwrap();assert_eq!((w,ht),(80,24));
        assert!((i16::from(pixels[0])-i16::from(value)).abs()<=2);
        assert_eq!(&bytes[..2],&[255,216]);assert_eq!(&bytes[n-2..],&[255,217]);
        assert_eq!(h.inner.ram[0x4000-1],0xa5);assert_eq!(h.inner.ram[0x4000+n],0xa5);
        assert_eq!(read(&mut d,&mut h,0x24),READY|DONE);
        write(&mut d,&mut h,BASE+0x24,READY|DONE).unwrap();
        assert_eq!(read(&mut d,&mut h,0x24),READY);assert!(!h.inner.irq);
        assert!(d.checkpoint().is_err());streams.push(bytes);
    }
    assert_ne!(streams[0],streams[1]);
}
#[test]
fn reset_cancels_and_busy_writes_refuse() {
    let (mut d,mut h)=fixture(32);let before=h.inner.ram.clone();
    write(&mut d,&mut h,BASE,1).unwrap();
    assert!(write(&mut d,&mut h,BASE+0x44,0).is_err());
    write(&mut d,&mut h,BASE,0x100).unwrap();
    h.inner.now=100;advance(&mut d,&mut h).unwrap();
    assert_eq!(h.inner.ram,before);assert!(h.effects.is_empty());
    assert_eq!(d.next_deadline(100),None);assert_eq!(read(&mut d,&mut h,0x94),0);
}
#[test]
fn invalid_configuration_never_publishes() {
    for (o,v) in [(0x130,1),(0x12c,8),(0x48,17),(0x80,9),(0x110,8),(0x10c,16)] {
        let (mut d,mut h)=fixture(32);let before=h.inner.ram.clone();
        write(&mut d,&mut h,BASE+o,v).unwrap();
        assert!(write(&mut d,&mut h,BASE,1).is_err(),"offset {o:x}");
        assert_eq!(d.next_deadline(0),None);assert_eq!(h.inner.ram,before);
        assert!(h.effects.is_empty());assert_eq!(read(&mut d,&mut h,0x94),0);
    }
    let (mut d,mut h)=fixture(32);d.law=None;
    assert!(write(&mut d,&mut h,BASE,1).unwrap_err().contains("explicit encoding law"));
    assert!(write(&mut d,&mut h,ADDRESS_HIGH,4).is_err());
    assert!(write(&mut d,&mut h,BASE+0x84,1).is_err());
}
#[test]
fn capacity_failure_at_completion_has_no_dma_or_irq() {
    let (mut d,mut h)=fixture(32);
    // The header fits but the entropy body does not.
    write(&mut d,&mut h,BASE+0x110,600).unwrap();
    let before=h.inner.ram.clone();
    write(&mut d,&mut h,BASE,1).unwrap();
    h.inner.now=10;assert!(advance(&mut d,&mut h).is_err());
    assert_eq!(h.inner.ram,before);assert!(h.effects.is_empty());
    assert_eq!(read(&mut d,&mut h,0x94),0);
}

#[test]
fn dma_errors_do_not_publish_completion() {
    for (offset, at_start) in [(0x100, true), (0x10c, false)] {
        let (mut d,mut h)=fixture(32);
        write(&mut d,&mut h,BASE+offset,0x20000).unwrap();
        let before=h.inner.ram.clone();
        let result=write(&mut d,&mut h,BASE,1);
        if at_start { assert!(result.is_err()); }
        else { result.unwrap();h.inner.now=10;assert!(advance(&mut d,&mut h).is_err()); }
        assert!(d.pending.is_none());assert_eq!(h.inner.ram,before);
        assert!(h.effects.is_empty());assert_eq!(read(&mut d,&mut h,0x94),0);
        assert_eq!(read(&mut d,&mut h,0x24)&DONE,0);
    }
}
#[test]
fn incomplete_or_overlong_table_upload_refuses() {
    let (mut d,mut h)=fixture(32);
    write(&mut d,&mut h,BASE+0x84,0x100).unwrap();
    write(&mut d,&mut h,BASE+0x8c,0x03030303).unwrap();
    assert!(write(&mut d,&mut h,BASE+0x84,1).is_err());
    assert!(write(&mut d,&mut h,BASE,1).is_err());
    for _ in 1..32 { write(&mut d,&mut h,BASE+0x8c,0x03030303).unwrap(); }
    assert!(write(&mut d,&mut h,BASE+0x8c,0x03030303).is_err());
    assert!(d.pending.is_none());assert!(h.effects.is_empty());
}
#[test]
fn uploaded_table_words_keep_big_endian_byte_order() {
    let (mut d,mut h)=fixture(32);
    write(&mut d,&mut h,BASE+0x84,0x100).unwrap();
    for i in 0..32u32 {
        let bytes=[1+i as u8,33+i as u8,65+i as u8,97+i as u8];
        write(&mut d,&mut h,BASE+0x8c,u32::from_be_bytes(bytes)).unwrap();
        assert_eq!(&d.tables[i as usize*4..][..4],&bytes);
    }
    write(&mut d,&mut h,BASE+0x84,1).unwrap();
    assert_eq!(d.tables.len(),128);
}
#[test]
fn malformed_accesses_refuse_without_state_change() {
    let (mut d,mut h)=fixture(32);let before=d.registers;
    for (a,size,value) in [(BASE+1,4,0),(BASE+0x44,2,0),
        (ADDRESS_HIGH+1,4,1),(ADDRESS_HIGH,8,1),(BASE+WINDOW,4,0),
        (BASE+0x44,4,u64::from(u32::MAX)+1)] {
        let mut cx=Cx::new(&mut h,0,0,0);
        d.write(&mut cx,a,size,value);assert!(cx.refusal().is_some());
        assert_eq!(d.registers,before);assert!(d.pending.is_none());
    }
}

#[test]
fn stock_reset_and_status_ack_preserve_idle_readiness() {
    let (mut d,mut h)=fixture(32);
    write(&mut d,&mut h,BASE+0x24,READY).unwrap();
    write(&mut d,&mut h,BASE,0x100).unwrap();
    assert_eq!(read(&mut d,&mut h,0x24),READY);
    assert!(write(&mut d,&mut h,BASE+0x24,1).is_err());
    assert!(write(&mut d,&mut h,BASE+0x24,READY|DONE).is_ok());
}
