//! Replays a TASVideos FCEUX movie (.fm2) in a libretro core and records what
//! a player would see next to the buttons pressed on every frame: training
//! pairs for the inverse-dynamics model (frames -> presses). Inputs come from
//! the movie file; nothing is read from the emulator but its picture.
//! tas_replay CORE ROM MOVIE.fm2 OUTPUT_DIR [--header HEX32] [--check N]
//! Writes OUTPUT_DIR/video.mkv (lossless-ish, native size, every frame),
//! OUTPUT_DIR/inputs.txt (one RLDUTSBA line per video frame, '.' = up) and,
//! with --check N, a PNG every N frames to eyeball sync.
//! Headerless ROMs (No-Intro headerless set) need --header with the 16-byte
//! iNES header as hex; zipped ROMs are read as `archive.zip::member`.
use anyhow::{Context, Result, bail, ensure};
use libloading::Library;
use std::{
    cell::RefCell,
    ffi::{CString, c_char, c_void},
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
};

#[derive(Default)]
struct Capture {
    pixels: Vec<u8>,
    size: (u32, u32),
    format: u32,
    buttons: u16,
}
thread_local! {static CAPTURE: RefCell<Capture> = RefCell::new(Capture::default());}

unsafe extern "C" fn environment(command: u32, data: *mut c_void) -> bool {
    match command {
        // SET_PIXEL_FORMAT
        10 => {
            CAPTURE.with(|s| s.borrow_mut().format = unsafe { *(data as *const u32) });
            true
        }
        // GET_CAN_DUPE
        3 => {
            unsafe { *(data as *mut bool) = true };
            true
        }
        // GET_VARIABLE_UPDATE
        17 => {
            unsafe { *(data as *mut bool) = false };
            true
        }
        52 => {
            unsafe { *(data as *mut u32) = 0 };
            true
        }
        // System and save directories
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
        let format = state.format;
        let mut pixels = Vec::with_capacity((width * height * 3) as usize);
        for y in 0..height as usize {
            let row = unsafe { (data as *const u8).add(y * pitch) };
            for x in 0..width as usize {
                let rgb = if format == 1 {
                    let p = unsafe { std::ptr::read_unaligned(row.add(x * 4) as *const u32) };
                    [(p >> 16) as u8, (p >> 8) as u8, p as u8]
                } else {
                    let p = unsafe { std::ptr::read_unaligned(row.add(x * 2) as *const u16) };
                    if format == 2 {
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
                pixels.extend_from_slice(&rgb);
            }
        }
        state.pixels = pixels;
        state.size = (width, height);
    });
}
unsafe extern "C" fn audio(_: i16, _: i16) {}
unsafe extern "C" fn audio_batch(_: *const i16, frames: usize) -> usize {
    frames
}
unsafe extern "C" fn poll() {}
unsafe extern "C" fn input(port: u32, device: u32, _: u32, id: u32) -> i16 {
    if port == 0 && device == 1 && id < 16 {
        CAPTURE.with(|s| ((s.borrow().buttons >> id) & 1) as i16)
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

/// The movie's frames: (reset command, RetroPad bits, FCEUX RLDUTSBA string
/// when the movie is an .fm2). A `.pads` file is any movie already turned
/// into RetroPad bits by scripts/player-tas-collect.py: one hex mask per
/// frame, `!` in front for a reset before that frame.
fn movie(path: &str) -> Result<Vec<(u32, u16, Option<String>)>> {
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut frames = vec![];
    if path.ends_with(".pads") {
        for line in text.lines() {
            let (command, hex) = match line.strip_prefix('!') {
                Some(rest) => (1, rest),
                None => (0, line),
            };
            frames.push((command, u16::from_str_radix(hex.trim(), 16)?, None));
        }
        return Ok(frames);
    }
    for line in text.lines().filter(|l| l.starts_with('|')) {
        let fields: Vec<&str> = line.split('|').collect();
        let command: u32 = fields.get(1).context("Movie line")?.trim().parse()?;
        let pad = fields.get(2).context("Movie pad")?.to_owned();
        ensure!(pad.len() == 8, "Pad 1 must be a standard controller");
        frames.push((command, retropad(pad), Some(pad.to_owned())));
    }
    Ok(frames)
}

/// RLDUTSBA (FCEUX order) to libretro RetroPad bits.
fn retropad(pad: &str) -> u16 {
    // libretro ids: B=0 SELECT=2 START=3 UP=4 DOWN=5 LEFT=6 RIGHT=7 A=8
    const IDS: [u16; 8] = [7, 6, 5, 4, 3, 2, 0, 8];
    pad.chars()
        .zip(IDS)
        .filter(|(c, _)| *c != '.' && *c != ' ')
        .fold(0, |bits, (_, id)| bits | 1 << id)
}

fn rom(spec: &str, header: Option<&str>) -> Result<Vec<u8>> {
    let mut bytes = match spec.split_once("::") {
        Some((archive, member)) => {
            let out = Command::new("unzip")
                .args(["-p", archive, member])
                .output()?;
            ensure!(out.status.success(), "unzip {archive}");
            out.stdout
        }
        None => std::fs::read(spec)?,
    };
    if let Some(hex) = header {
        ensure!(hex.len() == 32, "Header is 16 bytes of hex");
        let mut full = (0..16)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16))
            .collect::<Result<Vec<u8>, _>>()?;
        full.append(&mut bytes);
        bytes = full;
    }
    Ok(bytes)
}

unsafe fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    ensure!(
        args.len() >= 5,
        "Usage: tas_replay CORE ROM MOVIE.fm2|.pads OUTPUT_DIR [--header HEX32] [--ext nes] [--fps 60] [--check N]"
    );
    let flag = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };
    let check: u64 = flag("--check").map_or(Ok(0), str::parse)?;
    let directory = PathBuf::from(&args[4]);
    std::fs::create_dir_all(&directory)?;
    let frames = movie(&args[3])?;
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
    let rom = rom(&args[2], flag("--header"))?;
    // Cores open the ROM by path as well as taking its bytes.
    let file = directory.join(format!("game.{}", flag("--ext").unwrap_or("nes")));
    let fps = flag("--fps").unwrap_or("60");
    std::fs::write(&file, &rom)?;
    let rompath = CString::new(file.to_str().context("Path")?)?;
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
    let mut inputs = std::io::BufWriter::new(std::fs::File::create(directory.join("pads.txt"))?);
    let mut fceux = match frames.first() {
        Some((_, _, Some(_))) => Some(std::io::BufWriter::new(std::fs::File::create(
            directory.join("inputs.txt"),
        )?)),
        _ => None,
    };
    let mut encoder: Option<std::process::Child> = None;
    // The video keeps the first frame's size; cores that change resolution
    // mid-game (SNES hi-res, Genesis 256/320 wide) are scaled to it.
    let mut canvas = None;
    for (index, (command, buttons, pad)) in frames.iter().enumerate() {
        match command {
            0 => {}
            1 => call!("retro_reset", unsafe extern "C" fn()),
            // A power cycle at the very start is how most movies begin.
            2 if index == 0 => {}
            2 => call!("retro_reset", unsafe extern "C" fn()),
            other => bail!("Unsupported movie command {other} at frame {index}"),
        }
        CAPTURE.with(|s| s.borrow_mut().buttons = *buttons);
        call!("retro_run", unsafe extern "C" fn());
        // A core may skip drawing a frame that didn't change (a dupe); the
        // previous picture stands for it.
        let (size, pixels) = CAPTURE.with(|s| {
            let s = s.borrow();
            (s.size, s.pixels.clone())
        });
        ensure!(!pixels.is_empty(), "No picture at frame {index}");
        let canvas = *canvas.get_or_insert(size);
        let (size, pixels) = if size == canvas {
            (size, pixels)
        } else {
            let frame = image::RgbImage::from_raw(size.0, size.1, pixels).context("Frame size")?;
            let scaled = image::imageops::resize(
                &frame,
                canvas.0,
                canvas.1,
                image::imageops::FilterType::Nearest,
            );
            (canvas, scaled.into_raw())
        };
        let child = match &mut encoder {
            Some(child) => child,
            None => encoder.insert(
                Command::new("ffmpeg")
                    .args(["-nostdin", "-loglevel", "error", "-y", "-f", "rawvideo"])
                    .args(["-pix_fmt", "rgb24", "-s", &format!("{}x{}", size.0, size.1)])
                    .args(["-r", fps, "-i", "-", "-c:v", "libx264", "-preset", "fast"])
                    .args(["-crf", "10", "-pix_fmt", "yuv444p"])
                    .arg(directory.join("video.mkv"))
                    .stdin(Stdio::piped())
                    .spawn()
                    .context("ffmpeg")?,
            ),
        };
        child
            .stdin
            .as_mut()
            .context("ffmpeg stdin")?
            .write_all(&pixels)?;
        writeln!(inputs, "{buttons:04x}")?;
        if let (Some(f), Some(pad)) = (fceux.as_mut(), pad) {
            writeln!(f, "{pad}")?;
        }
        if check > 0 && (index as u64).is_multiple_of(check) {
            image::RgbImage::from_raw(size.0, size.1, pixels)
                .context("Frame size")?
                .save(directory.join(format!("check-{index:06}.png")))?;
        }
    }
    inputs.flush()?;
    if let Some(mut f) = fceux {
        f.flush()?;
    }
    if let Some(mut child) = encoder {
        drop(child.stdin.take());
        ensure!(child.wait()?.success(), "ffmpeg failed");
    }
    call!("retro_unload_game", unsafe extern "C" fn());
    std::fs::remove_file(&file)?;
    call!("retro_deinit", unsafe extern "C" fn());
    println!("{} frames -> {}", frames.len(), directory.display());
    Ok(())
}

fn main() -> Result<()> {
    unsafe { run() }
}
