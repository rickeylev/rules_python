#[cfg(not(windows))]
use std::cell::UnsafeCell;
use std::env;
use std::process;
#[cfg(not(windows))]
use std::ptr;

#[cfg(windows)]
use stage1_native_bootstrap_lib::{
    load_eof_trailer_config, resolve_self_executable, run_stage1_with_config,
    BootstrapConfig, CFG_MAGIC, CFG_MAGIC_LEN,
};
#[cfg(not(windows))]
use stage1_native_bootstrap_lib::{
    parse_cfg_payload, run_stage1_with_config, BootstrapConfig, CFG_BUFFER_SIZE,
    CFG_HEADER_LEN, CFG_MAGIC, CFG_MAGIC_LEN,
};

#[cfg(not(windows))]
#[repr(C, align(64))]
pub struct BootstrapCfgBuffer(pub UnsafeCell<[u8; CFG_BUFFER_SIZE]>);

#[cfg(not(windows))]
unsafe impl Sync for BootstrapCfgBuffer {}

#[cfg(not(windows))]
const fn init_cfg_buffer() -> BootstrapCfgBuffer {
    let mut buf = [0u8; CFG_BUFFER_SIZE];
    let mut i = 0;
    while i < CFG_MAGIC_LEN {
        buf[i] = CFG_MAGIC[i];
        i += 1;
    }
    BootstrapCfgBuffer(UnsafeCell::new(buf))
}

#[cfg(not(windows))]
#[used]
#[no_mangle]
pub static G_BOOTSTRAP_CFG: BootstrapCfgBuffer = init_cfg_buffer();

#[cfg(windows)]
#[used]
#[no_mangle]
pub static G_BOOTSTRAP_CFG: [u8; CFG_MAGIC_LEN] = CFG_MAGIC;

#[cfg(not(windows))]
fn load_embedded_config(_argv0: &str) -> Result<BootstrapConfig, String> {
    let base = std::hint::black_box(G_BOOTSTRAP_CFG.0.get() as *const u8);
    let b0 = unsafe { ptr::read_volatile(base.add(32)) } as u32;
    let b1 = unsafe { ptr::read_volatile(base.add(33)) } as u32;
    let b2 = unsafe { ptr::read_volatile(base.add(34)) } as u32;
    let b3 = unsafe { ptr::read_volatile(base.add(35)) } as u32;
    let payload_len = (b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)) as usize;
    if payload_len == 0 || payload_len > CFG_BUFFER_SIZE - CFG_HEADER_LEN {
        return Err("unconfigured native bootstrap template binary".to_string());
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

#[cfg(windows)]
fn load_embedded_config(argv0: &str) -> Result<BootstrapConfig, String> {
    let _ = std::hint::black_box(G_BOOTSTRAP_CFG.as_ptr());
    let self_exe = resolve_self_executable(argv0)?;
    load_eof_trailer_config(&self_exe)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let argv0 = args
        .first()
        .map(|s| s.as_str())
        .unwrap_or("stage1_native_bootstrap");
    let cfg = match load_embedded_config(argv0) {
        Ok(c) => c,
        Err(msg) => {
            if args.get(1).map(|s| s.as_str()) == Some("--template") {
                if let Err(err) = stage1_native_bootstrap_lib::run_embedder(args) {
                    eprintln!("stage1_native_bootstrap: {err}");
                    process::exit(1);
                }
                return;
            }
            eprintln!("stage1_native_bootstrap: {msg}");
            process::exit(1);
        }
    };
    let code = run_stage1_with_config(cfg, &args);
    process::exit(code);
}
