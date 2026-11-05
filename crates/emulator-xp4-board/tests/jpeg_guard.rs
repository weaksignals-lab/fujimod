use fujimod_emulator_xp4_board::{Definition, Session};
use serde_json::json;
fn session() -> Session {
    let d: Definition=serde_json::from_value(json!({"ram_size":65536,"cores":1,"discovery":false,
        "cbar":0xffff0000u32,"xp4_jpeg":{"raw_control_48":0,"raw_control_80":4,"restart_mcus":3,"completion_ticks":10},
        "initial_registers":[[[{"Arm32":{"R":15}},4096],[{"Arm32":{"R":14}},65280],
            [{"Arm32":{"R":0}},0xfffa0118u32],[{"Arm32":{"R":1}},1]]]})).unwrap();
    let s=Session::new(d,None,vec![]).unwrap();
    let code:Vec<_>=[0xe5801000u32,0xe12fff1e].into_iter().flat_map(u32::to_le_bytes).collect();
    s.write(0x1000,&code).unwrap();s
}
#[test]
fn connected_registers_are_separate_from_executable_backing() {
    let s=session();
    assert!(s.driver_guard_begin(vec![(0xfffa0000,0xfffa0004)],vec![],vec![],0xff00,16,0).is_err());
    s.driver_guard_begin(vec![(0x1000,0x1008)],vec![(0x4000,0x4100)],vec![(0x8000,0x9000)],0xff00,16,0).unwrap();
    s.run(0,8).unwrap();
    let g=s.driver_guard_state().unwrap();assert!(g.returned);assert!(g.failure.is_none());
    assert_eq!(g.mmio,vec![(0xfff4a740,0xfff4a748),(0xfffa0000,0xfffa0134)]);
    s.driver_guard_end().unwrap();
    // Pending-device access permissions cannot be broadened after return.
    assert!(s.driver_guard_begin(vec![(0x1000,0x1008)],vec![(0x4000,0x4200)],vec![(0x8000,0x9000)],0xff00,16,0).is_err());
}
#[test]
fn undeclared_register_window_refuses_before_guest_store() {
    let s=session();s.write_register(0,fujimod_emulator_cpu::arm::CORE[0],0xfffa0200).unwrap();
    s.driver_guard_begin(vec![(0x1000,0x1008)],vec![],vec![],0xff00,16,0).unwrap();
    let _=s.run(0,8);let g=s.driver_guard_state().unwrap();
    assert!(!g.returned);assert!(g.failure.unwrap().contains("rejected write"));
}
fn register_write(s: &Session, address: u64, value: u64) -> Result<(),String> {
    use fujimod_emulator_cpu::arm;
    s.write_register(0,arm::PC,0x1000)?;
    s.write_register(0,arm::CORE[14],0xff00)?;
    s.write_register(0,arm::CORE[0],address)?;
    s.write_register(0,arm::CORE[1],value)?;
    s.driver_guard_begin(vec![(0x1000,0x1008)],vec![(0x4000,0x5080)],vec![(0x8000,0xa000)],0xff00,128,0)?;
    let result=s.run(0,8);
    if let Some(error)=s.driver_guard_state().unwrap().failure { return Err(error); }
    result?;
    s.driver_guard_end()
}
#[test]
fn deferred_dma_keeps_bounds_after_submitting_call_returns() {
    use fujimod_emulator_cpu::arm;
    let s=session();
    s.write(0x1100,&0xeafffffeu32.to_le_bytes()).unwrap();
    s.write(0x4000,&vec![128;4224]).unwrap();
    for (o,v) in [(0x118,1),(0xc,0xa0003),(0x28,0xfd0000),(0x40,1),
        (0x44,(24<<16)|80),(0x80,4),(0x130,88),(0x100,0x4000),
        (0x10c,0x1000),(0x110,8192),(0x11c,1),(0x84,0x100)] {
        register_write(&s,0xfffa0000+o,v).unwrap();
    }
    for _ in 0..32 { register_write(&s,0xfffa008c,0x03030303).unwrap(); }
    register_write(&s,0xfffa0084,1).unwrap();
    let mut code=[0;8];s.read(0x1000,&mut code).unwrap();
    // Delay long enough for the START routine to return before DMA completion.
    // The selected board law has ten ticks; START is a store and BX LR.
    register_write(&s,0xfffa0000,1).unwrap();
    assert!(s.driver_guard_state().is_none());
    s.write_register(0,arm::PC,0x1100).unwrap();
    let _=s.run(0,32);
    assert!(s.state(false).failure.unwrap().contains("Driver DMA guard rejected write"));
    let mut after=[0;8];s.read(0x1000,&mut after).unwrap();assert_eq!(after,code);
}
