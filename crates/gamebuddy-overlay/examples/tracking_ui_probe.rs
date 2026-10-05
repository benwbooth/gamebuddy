//! Replay real captured frames through XComposite, the scheduler, and Qt's map.
//! The accompanying CDP observer measures the rendered marker, not just inference.
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use gamebuddy_services::{Store, navigation, screen};
    use serde_json::{Value, json};
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };
    use x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _};
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--prepare") {
        let source = Store::open(args.get(1).context("Source store")?)?;
        let root = PathBuf::from(args.get(2).context("New private store")?);
        ensure!(!root.exists(), "Use a new private store");
        let store = Store::open(&root)?;
        let id: String = source.get("active_guide")?.context("Active guide")?;
        let guide = source.guide(&id)?;
        store.save_guide(&guide)?;
        for key in [
            "active_guide".into(),
            "spatial-worker".into(),
            format!("world:{id}"),
            format!("world-status:{id}"),
            format!("world-profile:{id}"),
            format!("world-map-status:{id}"),
            format!("spatial-maps:{id}"),
            format!("atlas:{id}"),
        ] {
            if let Some(value) = source.get::<Value>(&key)? {
                store.put(&key, &value)?;
            }
        }
        std::fs::create_dir_all(root.join("maps"))?;
        for map in navigation::atlas(&store, &guide)?.maps {
            let path = navigation::cached_map_path(&source, &map);
            if path.exists() {
                std::fs::copy(path, navigation::cached_map_path(&store, &map))?;
            }
        }
        // This measures local map tracking without issuing LLM requests.
        let mut settings = gamebuddy_services::Settings::default();
        settings.reasoning.endpoint = "http://127.0.0.1:9".into();
        settings.vision.endpoint = "http://127.0.0.1:9".into();
        store.put("settings", &settings)?;
        store.put(
            "widget-layout",
            &json!({"location-map":{"x":700,"y":30,"width":540,"height":640}}),
        )?;
        return Ok(());
    }
    let root = PathBuf::from(args.first().context("Private store")?);
    ensure!(
        root.starts_with("/tmp"),
        "Only private test stores are allowed"
    );
    let frames = &args[1..];
    ensure!(
        frames.len() >= 2,
        "Supply at least two different captured game frames"
    );
    let (conn, index) = x11rb::connect(None)?;
    let display = &conn.setup().roots[index];
    let atom = |name| gamebuddy_overlay::x11::atom(&conn, name);
    ensure!(
        conn.get_property(
            false,
            display.root,
            atom("GAMESCOPE_XWAYLAND_SERVER_ID")?,
            AtomEnum::CARDINAL,
            0,
            1
        )?
        .reply()?
        .value_len
            == 1,
        "Run inside GameBuddy's isolated Gamescope session"
    );
    let window = conn.generate_id()?;
    conn.create_window(
        display.root_depth,
        window,
        display.root,
        0,
        0,
        1280,
        720,
        0,
        WindowClass::INPUT_OUTPUT,
        display.root_visual,
        &CreateWindowAux::new().background_pixel(0),
    )?
    .check()?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atom("_NET_WM_PID")?,
        AtomEnum::CARDINAL,
        &[std::process::id()],
    )?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"GameBuddy tracking latency replay",
    )?;
    conn.map_window(window)?.check()?;
    let gc = conn.generate_id()?;
    conn.create_gc(gc, window, &CreateGCAux::new())?.check()?;
    conn.flush()?;
    let store = Store::open(&root)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let ticket = loop {
        if let Some(session) = screen::status(&store)?["session_id"].as_str()
            && let Ok(ticket) = screen::ticket(&store, session)
        {
            break ticket;
        }
        ensure!(Instant::now() < deadline, "Capture did not start");
        std::thread::sleep(Duration::from_millis(50));
    };
    for n in 0..6 {
        let path = &frames[n % frames.len()];
        let image = image::open(path)?.thumbnail(1280, 720).to_rgb8();
        let (w, h) = image.dimensions();
        let (x, y) = ((1280 - w) / 2, (720 - h) / 2);
        if n == 0 {
            screen::set_crop(
                &store,
                &ticket,
                gamebuddy_core::Crop {
                    x: x as f32 / 1280.,
                    y: y as f32 / 720.,
                    width: w as f32 / 1280.,
                    height: h as f32 / 720.,
                },
            )?;
        }
        for row in (0..h).step_by(32) {
            let height = (h - row).min(32);
            let mut bytes = Vec::new();
            for py in row..row + height {
                for px in 0..w {
                    let p = image.get_pixel(px, py).0;
                    bytes.extend([p[2], p[1], p[0], 0]);
                }
            }
            conn.put_image(
                ImageFormat::Z_PIXMAP,
                window,
                gc,
                w as u16,
                height as u16,
                x as i16,
                (y + row) as i16,
                0,
                display.root_depth,
                &bytes,
            )?
            .check()?;
        }
        conn.get_input_focus()?.reply()?;
        let at = screen::now_ms();
        std::fs::write(
            root.join("frame.pending"),
            json!({"index":n,"path":path,"displayed_at_ms":at}).to_string(),
        )?;
        std::fs::rename(root.join("frame.pending"), root.join("frame.json"))?;
        println!("GAMEBUDDY_REPLAY_FRAME index={n} at_ms={at}");
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            if std::fs::read_to_string(root.join("advance"))
                .ok()
                .as_deref()
                == Some(&n.to_string())
            {
                break;
            }
            ensure!(
                Instant::now() < deadline,
                "No rendered map marker for frame {n}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(350));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This capture probe requires Linux/Gamescope");
}
