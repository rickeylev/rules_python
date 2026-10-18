use std::cell::UnsafeCell;
use std::env;
use std::process;
use std::ptr;

use stage1_native_bootstrap_lib::{
    parse_cfg_payload, run_stage1_with_config, BootstrapConfig, CFG_BUFFER_SIZE,
    CFG_HEADER_LEN, CFG_MAGIC, CFG_MAGIC_LEN,
};

#[repr(C, align(64))]
pub struct BootstrapCfgBuffer(pub UnsafeCell<[u8; CFG_BUFFER_SIZE]>);

unsafe impl Sync for BootstrapCfgBuffer {}

const fn init_cfg_buffer() -> BootstrapCfgBuffer {
    let mut buf = [0u8; CFG_BUFFER_SIZE];
    let mut i = 0;
    while i < CFG_MAGIC_LEN {
        buf[i] = CFG_MAGIC[i];
        i += 1;
    }
    BootstrapCfgBuffer(UnsafeCell::new(buf))
}

#[used]
#[no_mangle]
pub static G_BOOTSTRAP_CFG: BootstrapCfgBuffer = init_cfg_buffer();

fn load_embedded_config() -> Result<BootstrapConfig, &'static str> {
    let base = std::hint::black_box(G_BOOTSTRAP_CFG.0.get() as *const u8);
    let b0 = unsafe { ptr::read_volatile(base.add(32)) } as u32;
    let b1 = unsafe { ptr::read_volatile(base.add(33)) } as u32;
    let b2 = unsafe { ptr::read_volatile(base.add(34)) } as u32;
    let b3 = unsafe { ptr::read_volatile(base.add(35)) } as u32;
    let payload_len = (b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) as usize;
    if payload_len == 0 || payload_len > CFG_BUFFER_SIZE - CFG_HEADER_LEN {
        return Err("unconfigured native bootstrap template binary");
    }

    let mut payload = Vec::with_capacity(payload_len);
    for i in 0..payload_len {
        let b = unsafe { ptr::read_volatile(base.add(CFG_HEADER_LEN + i)) };
        payload.push(b);
    }

    let mut cfg = BootstrapConfig::default();
    parse_cfg_payload(&payload, &mut cfg);
    Ok(cfg)
}

fn main() {
    let cfg = match load_embedded_config() {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("stage1_native_bootstrap: {msg}");
            process::exit(1);
        }
    };
    let args: Vec<String> = env::args().collect();
    let code = run_stage1_with_config(cfg, &args);
    process::exit(code);
}
