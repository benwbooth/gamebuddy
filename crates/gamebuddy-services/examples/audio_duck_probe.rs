//! Isolated PipeWire integration test. Never routes audio to a physical device.
#[cfg(target_os = "linux")]
mod probe {
    use anyhow::{Context, Result, ensure};
    use serde_json::{Value, json};
    use std::{
        io::{BufRead, Read, Write},
        process::{Child, Command, Stdio},
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    struct Owned(Child);
    impl Drop for Owned {
        fn drop(&mut self) {
            if let Some(mut input) = self.0.stdin.take() {
                let _ = writeln!(input, "stop");
                drop(input);
                let until = Instant::now() + Duration::from_secs(3);
                while Instant::now() < until && self.0.try_wait().ok().flatten().is_none() {
                    thread::sleep(Duration::from_millis(20));
                }
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn pactl(args: &[&str]) -> Result<String> {
        let o = Command::new("pactl").args(args).output()?;
        ensure!(
            o.status.success(),
            "pactl: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        Ok(String::from_utf8(o.stdout)?)
    }
    struct Sink(String);
    impl Drop for Sink {
        fn drop(&mut self) {
            let _ = pactl(&["unload-module", &self.0]);
        }
    }
    fn objects() -> Result<Value> {
        Ok(serde_json::from_slice(
            &Command::new("pw-dump").output()?.stdout,
        )?)
    }
    fn stream(pid: u32) -> Result<Option<Value>> {
        Ok(objects()?
            .as_array()
            .context("objects")?
            .iter()
            .find(|o| {
                o["info"]["props"]["media.class"] == "Stream/Output/Audio"
                    && o["info"]["props"]["application.process.id"]
                        .as_str()
                        .and_then(|s| s.parse().ok())
                        .or_else(|| {
                            o["info"]["props"]["application.process.id"]
                                .as_u64()
                                .map(|n| n as u32)
                        })
                        == Some(pid)
            })
            .cloned())
    }
    fn props(s: &Value) -> &Value {
        &s["info"]["params"]["Props"][0]
    }
    fn wait(
        mut beat: impl FnMut() -> Result<()>,
        mut check: impl FnMut() -> Result<bool>,
    ) -> Result<()> {
        let until = Instant::now() + Duration::from_secs(8);
        loop {
            beat()?;
            if check()? {
                return Ok(());
            }
            ensure!(Instant::now() < until, "audio condition timed out");
            thread::sleep(Duration::from_millis(100));
        }
    }
    fn player(sink: &str, audible: bool) -> Result<Owned> {
        let mut child = Command::new("paplay")
            .args([
                "--raw",
                "--rate=48000",
                "--format=s16le",
                "--channels=2",
                "--latency-msec=50",
                "--device",
                sink,
                "--client-name",
                "GameBuddyDuckingTest",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let mut input = child.stdin.take().unwrap();
        thread::spawn(move || {
            let bytes: Vec<_> = (0..4800)
                .flat_map(|i| {
                    let v = if audible {
                        ((i as f32 * std::f32::consts::TAU / 48.0).sin() * 6000.0) as i16
                    } else {
                        0
                    };
                    [v.to_le_bytes(), v.to_le_bytes()].concat()
                })
                .collect();
            while input.write_all(&bytes).is_ok() {}
        });
        Ok(Owned(child))
    }
    pub fn run() -> Result<()> {
        if std::env::args().nth(1).as_deref() == Some("--game") {
            let sink = std::env::args().nth(2).context("sink")?;
            let mut game = player(&sink, true)?;
            println!("{}", game.0.id());
            std::io::stdout().flush()?;
            for line in std::io::stdin().lock().lines() {
                if line? != "replace" {
                    break;
                }
                drop(game);
                game = player(&sink, true)?;
                println!("{}", game.0.id());
                std::io::stdout().flush()?;
            }
            return Ok(());
        }
        let name = format!("gamebuddy_duck_test_{}", std::process::id());
        let _sink = Sink(
            pactl(&[
                "load-module",
                "module-null-sink",
                &format!("sink_name={name}"),
            ])?
            .trim()
            .into(),
        );
        let mut game = Owned(
            Command::new(std::env::current_exe()?)
                .args(["--game", &name])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?,
        );
        let mut game_output = std::io::BufReader::new(game.0.stdout.take().unwrap());
        let mut line = String::new();
        game_output.read_line(&mut line)?;
        let mut pid: u32 = line.trim().parse()?;
        let other = player(&name, false)?;
        wait(
            || Ok(()),
            || Ok(stream(pid)?.is_some() && stream(other.0.id())?.is_some()),
        )?;
        let original = stream(pid)?.unwrap();
        let other_before = props(&stream(other.0.id())?.unwrap()).clone();
        let node = original["id"].as_u64().unwrap().to_string();
        let set = Command::new("pw-cli")
            .args([
                "set-param",
                &node,
                "Props",
                r#"{"channelVolumes":[0.8,0.4]}"#,
            ])
            .status()?;
        ensure!(set.success(), "set fixture balance");
        let mut recording = Owned(
            Command::new("parec")
                .args([
                    "--device",
                    &format!("{name}.monitor"),
                    "--raw",
                    "--format=s16le",
                    "--rate=48000",
                    "--channels=2",
                    "--latency-msec=50",
                ])
                .stdout(Stdio::piped())
                .spawn()?,
        );
        let mut audio = recording.0.stdout.take().unwrap();
        let level = Arc::new(Mutex::new(0.0_f64));
        let measured = level.clone();
        thread::spawn(move || {
            let mut bytes = [0_u8; 19200];
            while audio.read_exact(&mut bytes).is_ok() {
                let sum: f64 = bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|s| {
                        let v = i16::from_le_bytes([s[0], s[1]]) as f64;
                        v * v
                    })
                    .sum();
                *measured.lock().unwrap() = (sum / 9600.0).sqrt();
            }
        });
        wait(|| Ok(()), || Ok(*level.lock().unwrap() > 100.0))?;
        thread::sleep(Duration::from_millis(300));
        let baseline = *level.lock().unwrap();
        ensure!(baseline > 100.0, "no fixture audio");
        let mut helper = Owned(
            Command::new("target/debug/gamebuddy")
                .args(["--audio-duck-worker", &game.0.id().to_string()])
                .stdin(Stdio::piped())
                .stdout(Stdio::inherit())
                .spawn()?,
        );
        let input = helper.0.stdin.as_mut().unwrap();
        let lowered = |pid| -> Result<bool> {
            let Some(s) = stream(pid)? else {
                return Ok(false);
            };
            let normal = props(&s)["channelVolumes"][0].as_f64().unwrap_or(0.0);
            Ok(normal > 0.0 && props(&s)["softVolumes"][0].as_f64().unwrap_or(1.0) / normal < 0.3)
        };
        wait(
            || {
                writeln!(input, "1")?;
                Ok(())
            },
            || lowered(pid),
        )?;
        for _ in 0..10 {
            writeln!(input, "1")?;
            thread::sleep(Duration::from_millis(100));
        }
        let ratio = *level.lock().unwrap() / baseline;
        ensure!(
            (0.20..0.30).contains(&ratio),
            "measured attenuation {ratio}, expected -12 dB"
        );
        ensure!(
            props(&stream(pid)?.unwrap())["channelVolumes"] == json!([0.8, 0.4]),
            "saved game volume changed"
        );
        ensure!(
            *props(&stream(other.0.id())?.unwrap()) == other_before,
            "unrelated audio changed"
        );
        println!("Duck audio ratio={ratio:.3}; game balance and unrelated stream unchanged");
        wait(
            || {
                writeln!(input, "0")?;
                Ok(())
            },
            || Ok(*level.lock().unwrap() > baseline * 0.95),
        )?;
        println!("Silence restores measured output");
        wait(
            || {
                writeln!(input, "1")?;
                Ok(())
            },
            || lowered(pid),
        )?;
        let node = stream(pid)?.unwrap()["id"].as_u64().unwrap().to_string();
        Command::new("pw-cli")
            .args([
                "set-param",
                &node,
                "Props",
                r#"{"channelVolumes":[0.3,0.2]}"#,
            ])
            .status()?;
        for _ in 0..8 {
            writeln!(input, "1")?;
            thread::sleep(Duration::from_millis(100));
        }
        ensure!(
            props(&stream(pid)?.unwrap())["channelVolumes"] == json!([0.3, 0.2]),
            "manual change overwritten"
        );
        // Recreate the game's audio stream during the same conversation. This
        // must start from saved user volume, not compound the earlier ducking.
        writeln!(game.0.stdin.as_mut().unwrap(), "replace")?;
        line.clear();
        game_output.read_line(&mut line)?;
        pid = line.trim().parse()?;
        println!("Replacement stream PID {pid}");
        wait(
            || {
                writeln!(input, "1")?;
                Ok(())
            },
            || lowered(pid),
        )?;
        let current = stream(pid)?.unwrap();
        let normal = props(&current)["channelVolumes"][0].as_f64().unwrap();
        let soft = props(&current)["softVolumes"][0].as_f64().unwrap();
        ensure!(
            (soft / normal - 0.25118864).abs() < 0.002,
            "stream recreated with compounded attenuation"
        );
        // EOF simulates an abruptly terminated overlay while its game survives.
        helper.0.stdin.take();
        println!("Waiting for helper EOF restoration");
        wait(|| Ok(()), || Ok(helper.0.try_wait()?.is_some()))?;
        let current = stream(pid)?.unwrap();
        ensure!(
            (props(&current)["softVolumes"][0].as_f64().unwrap() - normal).abs() < 0.0001,
            "EOF failed to restore"
        );
        println!(
            "GAMEBUDDY_DUCK_TEST_PASS descendant ownership, measured -12 dB, silence, user changes, stream recreation, EOF restoration"
        );
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    probe::run()
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("PipeWire audio test runs on Linux");
}
