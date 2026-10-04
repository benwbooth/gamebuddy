//! Isolated software-rendered gameplay oracle. Never reads/writes player saves.
//! Usage: libretro_probe CORE ROM OUTPUT_DIR [INPUT_SCRIPT]
//! Script: idle:300,start:2,idle:180,right:1000,save:0,left:200,restore:0
use anyhow::{Context, Result, ensure};
use libloading::Library;
use serde_json::json;
use std::{
    cell::RefCell,
    ffi::{CString, c_char, c_void},
    io::Write,
    path::PathBuf,
};
#[derive(Default)]
struct Capture {
    pixels: Option<image::RgbImage>,
    format: u32,
    button: u32,
}
thread_local! {static CAPTURE:RefCell<Capture>=RefCell::new(Capture::default());}
unsafe extern "C" fn environment(command: u32, data: *mut c_void) -> bool {
    match command {
        10 => {
            CAPTURE.with(|s| s.borrow_mut().format = unsafe { *(data as *const u32) });
            true
        }
        3 => {
            unsafe { *(data as *mut bool) = true };
            true
        }
        17 => {
            unsafe { *(data as *mut bool) = false };
            true
        }
        52 => {
            unsafe { *(data as *mut u32) = 0 };
            true
        }
        9 | 31 => {
            unsafe { *(data as *mut *const c_char) = c"/tmp".as_ptr() };
            true
        }
        11 | 16 | 18 | 35 => true,
        _ => false,
    }
}
unsafe extern "C" fn video(data: *const c_void, width: u32, height: u32, pitch: usize) {
    if data.is_null() || data as usize == usize::MAX {
        return;
    }
    CAPTURE.with(|state| {
        let mut state = state.borrow_mut();
        let mut image = image::RgbImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let row = unsafe { (data as *const u8).add(y as usize * pitch) };
                let rgb = if state.format == 1 {
                    let p =
                        unsafe { std::ptr::read_unaligned(row.add(x as usize * 4) as *const u32) };
                    [(p >> 16) as u8, (p >> 8) as u8, p as u8]
                } else {
                    let p =
                        unsafe { std::ptr::read_unaligned(row.add(x as usize * 2) as *const u16) };
                    if state.format == 2 {
                        [
                            ((p >> 11) & 31) as u8 * 8,
                            ((p >> 5) & 63) as u8 * 4,
                            (p & 31) as u8 * 8,
                        ]
                    } else {
                        [
                            ((p >> 10) & 31) as u8 * 8,
                            ((p >> 5) & 31) as u8 * 8,
                            (p & 31) as u8 * 8,
                        ]
                    }
                };
                image.put_pixel(x, y, image::Rgb(rgb));
            }
        }
        state.pixels = Some(image);
    });
}
unsafe extern "C" fn audio(_: i16, _: i16) {}
unsafe extern "C" fn audio_batch(_: *const i16, frames: usize) -> usize {
    frames
}
unsafe extern "C" fn poll() {}
unsafe extern "C" fn input(port: u32, device: u32, _: u32, id: u32) -> i16 {
    if port == 0 && device == 1 && id < 16 {
        CAPTURE.with(|s| ((s.borrow().button >> id) & 1) as i16)
    } else {
        0
    }
}
#[repr(C)]
struct GameInfo {
    path: *const c_char,
    data: *const c_void,
    size: usize,
    meta: *const c_char,
}
unsafe fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    ensure!(
        args.len() >= 4,
        "Usage: libretro_probe CORE ROM OUTPUT [SCRIPT]"
    );
    let directory = PathBuf::from(&args[3]);
    std::fs::create_dir_all(&directory)?;
    let default_script = format!(
        "idle:180,start:2,idle:1000,right:100,idle:180,{},right:700,left:8,up:60,idle:180,right:800,up:30,idle:180,right:800,save:0,left:800,restore:0,idle:120",
        vec!["a:2,idle:120"; 25].join(",")
    );
    let script = args.get(4).map(String::as_str).unwrap_or(&default_script);
    let library = unsafe { Library::new(&args[1]) }?;
    macro_rules! call {($name:literal,$signature:ty $(,$arg:expr)*)=>{unsafe{library.get::<$signature>(concat!($name,"\0").as_bytes())?($($arg),*)}};}
    call!(
        "retro_set_environment",
        unsafe extern "C" fn(unsafe extern "C" fn(u32, *mut c_void) -> bool),
        environment
    );
    call!(
        "retro_set_video_refresh",
        unsafe extern "C" fn(unsafe extern "C" fn(*const c_void, u32, u32, usize)),
        video
    );
    call!(
        "retro_set_audio_sample",
        unsafe extern "C" fn(unsafe extern "C" fn(i16, i16)),
        audio
    );
    call!(
        "retro_set_audio_sample_batch",
        unsafe extern "C" fn(unsafe extern "C" fn(*const i16, usize) -> usize),
        audio_batch
    );
    call!(
        "retro_set_input_poll",
        unsafe extern "C" fn(unsafe extern "C" fn()),
        poll
    );
    call!(
        "retro_set_input_state",
        unsafe extern "C" fn(unsafe extern "C" fn(u32, u32, u32, u32) -> i16),
        input
    );
    call!("retro_init", unsafe extern "C" fn());
    let rom = std::fs::read(&args[2])?;
    let rompath = CString::new(args[2].as_str())?;
    let info = GameInfo {
        path: rompath.as_ptr(),
        data: rom.as_ptr().cast(),
        size: rom.len(),
        meta: std::ptr::null(),
    };
    ensure!(
        call!(
            "retro_load_game",
            unsafe extern "C" fn(*const GameInfo) -> bool,
            &info
        ),
        "Core could not load ROM"
    );
    let mut saved = vec![];
    let mut frame = 0u64;
    let mut epoch = 0u64;
    let mut record = std::fs::File::create(directory.join("oracle.jsonl"))?;
    for instruction in script.split(',') {
        let (action, count) = instruction
            .split_once(':')
            .context("Script entry needs action:frames")?;
        let count = count.parse::<u64>()?;
        if action == "save" {
            saved = vec![0; call!("retro_serialize_size", unsafe extern "C" fn() -> usize)];
            ensure!(
                call!(
                    "retro_serialize",
                    unsafe extern "C" fn(*mut c_void, usize) -> bool,
                    saved.as_mut_ptr().cast(),
                    saved.len()
                ),
                "Save failed"
            );
            continue;
        }
        if action == "restore" {
            ensure!(
                !saved.is_empty()
                    && call!(
                        "retro_unserialize",
                        unsafe extern "C" fn(*const c_void, usize) -> bool,
                        saved.as_ptr().cast(),
                        saved.len()
                    ),
                "Restore failed"
            );
            epoch += 1;
            continue;
        }
        let button = match action {
            "idle" => 0,
            "start" => 1 << 3,
            "right" => 1 << 7,
            "left" => 1 << 6,
            "up" => 1 << 4,
            "down" => 1 << 5,
            "a" => 1 << 8,
            "b" => 1,
            _ => anyhow::bail!("Unknown input"),
        };
        CAPTURE.with(|s| s.borrow_mut().button = button);
        for _ in 0..count {
            call!("retro_run", unsafe extern "C" fn());
            frame += 1;
            if !frame.is_multiple_of(30) {
                continue;
            }
            let size = call!(
                "retro_get_memory_size",
                unsafe extern "C" fn(u32) -> usize,
                2
            );
            let data = call!(
                "retro_get_memory_data",
                unsafe extern "C" fn(u32) -> *const u8,
                2
            );
            ensure!(
                size >= 2048 && !data.is_null(),
                "Core does not expose NES RAM"
            );
            let ram = unsafe { std::slice::from_raw_parts(data, 2048) };
            let state = gamebuddy_services::telemetry::decode_faxanadu(ram).ok();
            let name = format!("frame-{frame:06}.png");
            CAPTURE.with(|s| -> Result<()> {
                Ok(s.borrow()
                    .pixels
                    .as_ref()
                    .context("No video frame")?
                    .save(directory.join(&name))?)
            })?;
            writeln!(
                record,
                "{}",
                json!({"path":name,"at_ms":frame*1000/60,"session":"isolated-libretro","epoch":epoch,"input":action,"state":state})
            )?;
        }
    }
    call!("retro_unload_game", unsafe extern "C" fn());
    call!("retro_deinit", unsafe extern "C" fn());
    println!(
        "{}",
        json!({"frames":frame,"epochs":epoch+1,"output":directory})
    );
    Ok(())
}
fn main() -> Result<()> {
    unsafe { run() }
}
