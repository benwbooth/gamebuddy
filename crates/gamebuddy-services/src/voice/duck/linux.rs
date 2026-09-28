use super::Conversation;
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, Read, Write},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

// Temporary software attenuation leaves WirePlumber's saved channelVolumes
// untouched, including when the game destroys/recreates its audio stream.
const DUCK_GAIN: f64 = 0.251188643150958; // 10^(-12/20), linear amplitude

fn command(program: &str, args: &[&str]) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Game audio ducking requires {program}"))?;
    // Drain concurrently: a list of streams/clients can exceed a pipe buffer.
    let mut stdout = child.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        let mut text = String::new();
        stdout.read_to_string(&mut text).map(|_| text)
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            anyhow::bail!("Audio server timed out");
        }
        thread::sleep(Duration::from_millis(5));
    };
    let text = reader
        .join()
        .map_err(|_| anyhow::anyhow!("Audio reader stopped"))??;
    ensure!(status.success(), "Audio server rejected {}", args.join(" "));
    Ok(text)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Process {
    pid: u32,
    parent: u32,
    start: u64,
}
fn process(pid: u32) -> Option<Process> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The parenthesized command can contain spaces and closing parentheses.
    let fields: Vec<_> = stat.rsplit_once(") ")?.1.split_whitespace().collect();
    Some(Process {
        pid,
        parent: fields.get(1)?.parse().ok()?,
        start: fields.get(19)?.parse().ok()?,
    })
}
fn belongs_to(mut pid: u32, root: &Process) -> bool {
    for _ in 0..128 {
        let Some(p) = process(pid) else { return false };
        if p.pid == root.pid {
            return p.start == root.start;
        }
        if p.parent <= 1 || p.parent == pid {
            return false;
        }
        pid = p.parent;
    }
    false
}
fn number(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .or_else(|| value.as_str()?.parse().ok())
}

/// Pulse clients can report container-local PIDs. Resolve those only when the
/// namespace PID plus executable identifies ONE host process across all games.
/// Never match by app name alone (two RetroArch sessions may be running).
fn owner(properties: &Value, root: &Process) -> Option<Process> {
    let reported = number(&properties["application.process.id"]).or_else(|| {
        (properties["client.api"] != "pipewire-pulse")
            .then(|| number(&properties["pipewire.sec.pid"]))
            .flatten()
    })?;
    if belongs_to(reported, root) {
        return process(reported);
    }
    let binary = properties["application.process.binary"].as_str()?;
    if fs::read_link(format!("/proc/{reported}/exe"))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .as_deref()
        == Some(binary)
    {
        return None;
    }
    let mut matches = Vec::new();
    for entry in fs::read_dir("/proc").ok()?.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(exe) = fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if exe.file_name().and_then(|n| n.to_str()) != Some(binary) {
            continue;
        }
        let Ok(status) = fs::read_to_string(entry.path().join("status")) else {
            continue;
        };
        let ns: Vec<_> = status
            .lines()
            .find_map(|line| line.strip_prefix("NSpid:"))?
            .split_whitespace()
            .filter_map(|s| s.parse::<u32>().ok())
            .collect();
        if ns.len() > 1 && ns.last() == Some(&reported) {
            matches.push(pid)
        }
    }
    if matches.len() == 1 && belongs_to(matches[0], root) {
        process(matches[0])
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Identity {
    index: u32,
    client: u32,
    serial: String,
}
#[derive(Clone, Debug)]
struct Stream {
    id: Identity,
    properties: Value,
    volume: Vec<f64>,
    soft: Vec<f64>,
    muted: bool,
}
fn floats(value: &Value) -> Option<Vec<f64>> {
    let values: Vec<_> = value
        .as_array()?
        .iter()
        .map(Value::as_f64)
        .collect::<Option<_>>()?;
    (!values.is_empty()
        && values
            .iter()
            .all(|v| v.is_finite() && (0.0..=64.0).contains(v)))
    .then_some(values)
}
fn parse_streams(text: &str) -> Result<Vec<Stream>> {
    let objects: Value = serde_json::from_str(text)?;
    let objects = objects.as_array().context("Invalid PipeWire object list")?;
    let mut result = Vec::new();
    for object in objects {
        let properties = &object["info"]["props"];
        if properties["media.class"] != "Stream/Output/Audio" {
            continue;
        }
        let (Some(index), Some(client), Some(serial)) = (
            number(&object["id"]),
            number(&properties["client.id"]),
            properties["object.serial"].as_u64(),
        ) else {
            continue;
        };
        let mut combined = objects
            .iter()
            .find(|o| number(&o["id"]) == Some(client))
            .map(|o| o["info"]["props"].clone())
            .unwrap_or(json!({}));
        if let (Some(target), Some(extra)) = (combined.as_object_mut(), properties.as_object()) {
            target.extend(extra.clone())
        }
        let Some(props) = object["info"]["params"]["Props"]
            .as_array()
            .and_then(|v| v.first())
        else {
            continue;
        };
        let (Some(volume), Some(soft_array), Some(muted)) = (
            floats(&props["channelVolumes"]),
            props["softVolumes"].as_array(),
            props["mute"].as_bool(),
        ) else {
            continue;
        };
        // New Pulse streams can expose an empty soft-volume array until the
        // first software-volume override. Their effective levels are channels.
        let soft = if soft_array.is_empty() {
            volume.clone()
        } else {
            let Some(soft) = floats(&props["softVolumes"]) else {
                continue;
            };
            soft
        };
        if volume.len() != soft.len() {
            continue;
        }
        result.push(Stream {
            id: Identity {
                index,
                client,
                serial: serial.to_string(),
            },
            properties: combined,
            volume,
            soft,
            muted,
        });
    }
    Ok(result)
}
fn streams() -> Result<Vec<Stream>> {
    parse_streams(&command("pw-dump", &[])?)
}
#[derive(Debug)]
enum Change {
    Attenuate(Vec<f64>),
    Restore,
}
fn set_volume(stream: &Stream, change: Change) -> Result<()> {
    let props = match change {
        Change::Attenuate(values) => json!({"softVolumes":values,"softMute":stream.muted}),
        // Remove our temporary attenuation using the current user channel
        // levels. A later mixer edit selects ordinary channel control again.
        // Neither operation writes an old value into the user's saved volume.
        Change::Restore => json!({"softVolumes":stream.volume,"softMute":stream.muted}),
    };
    command(
        "pw-cli",
        &[
            "set-param",
            &stream.id.index.to_string(),
            "Props",
            &props.to_string(),
        ],
    )?;
    Ok(())
}
fn same_volume(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.00002)
}
struct Lease {
    owner: Process,
    original: Vec<f64>,
    applied: Vec<f64>,
    muted: bool,
}
#[derive(Default)]
struct Mixer {
    leases: HashMap<Identity, Lease>,
    overridden: HashSet<Identity>,
    gain: f64,
}
impl Mixer {
    fn new() -> Self {
        Self {
            gain: 1.0,
            ..Self::default()
        }
    }
    fn tick(
        &mut self,
        root: &Process,
        streams: &[Stream],
        duck: bool,
        immediate: bool,
        mut set: impl FnMut(&Stream, Change) -> Result<()>,
    ) -> Result<()> {
        let target = if duck { DUCK_GAIN } else { 1.0 };
        let rate: f64 = if duck { 0.3 } else { 0.13 };
        self.gain = if immediate {
            target
        } else {
            self.gain + (target - self.gain).clamp(-rate, rate)
        };
        self.leases
            .retain(|id, _| streams.iter().any(|s| &s.id == id));
        self.overridden
            .retain(|id| streams.iter().any(|s| &s.id == id));
        for stream in streams {
            if let Some(lease) = self.leases.get(&stream.id)
                && (process(lease.owner.pid).is_none_or(|p| p.start != lease.owner.start)
                    || !same_volume(&stream.volume, &lease.original)
                    || !same_volume(&stream.soft, &lease.applied))
            {
                // A user's volume adjustment already re-enables normal channel
                // volume control. Relinquish ownership until this conversation ends.
                self.leases.remove(&stream.id);
                self.overridden.insert(stream.id.clone());
            }
            if duck
                && !self.leases.contains_key(&stream.id)
                && !self.overridden.contains(&stream.id)
                && let Some(owner) = owner(&stream.properties, root)
            {
                self.leases.insert(
                    stream.id.clone(),
                    Lease {
                        owner,
                        original: stream.volume.clone(),
                        applied: stream.soft.clone(),
                        muted: stream.muted,
                    },
                );
            }
            if let Some(lease) = self.leases.get_mut(&stream.id) {
                if !duck && self.gain == 1.0 {
                    set(stream, Change::Restore)?;
                    self.leases.remove(&stream.id);
                } else {
                    let next: Vec<_> = lease.original.iter().map(|v| v * self.gain).collect();
                    if !same_volume(&stream.soft, &next) || lease.muted != stream.muted {
                        set(stream, Change::Attenuate(next.clone()))?;
                        lease.applied = next;
                        lease.muted = stream.muted;
                    }
                }
            }
        }
        if !duck && self.gain == 1.0 {
            self.overridden.clear()
        }
        Ok(())
    }
}

/// Internal helper protocol: each line renews a three-second lease. `1` means
/// conversation activity, `0` means idle, `stop` or EOF restores immediately.
pub fn run(pid: u32) -> Result<()> {
    ensure!(pid > 1 && pid != std::process::id(), "Invalid game PID");
    let root = process(pid).context("Game process exited")?;
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut mixer = Mixer::new();
    let mut conversation = Conversation::default();
    let mut active = false;
    let mut heartbeat = Instant::now();
    let mut previous = String::new();
    loop {
        let mut stop = false;
        loop {
            match rx.try_recv() {
                Ok(line) if line == "0" || line == "1" => {
                    active = line == "1";
                    heartbeat = Instant::now();
                }
                Ok(_) | Err(mpsc::TryRecvError::Disconnected) => {
                    stop = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        stop |= heartbeat.elapsed() > Duration::from_secs(3)
            || process(pid).is_none_or(|p| p.start != root.start);
        let duck = !stop && conversation.update(active, Instant::now());
        let result =
            streams().and_then(|streams| mixer.tick(&root, &streams, duck, stop, set_volume));
        let status = match result {
            Ok(()) => {
                if !mixer.leases.is_empty() {
                    "Game audio lowered during conversation".into()
                } else if duck {
                    "Waiting for the game's audio stream".into()
                } else {
                    "Game audio ducking ready".into()
                }
            }
            Err(e) => format!("Game audio: {e:#}"),
        };
        if status != previous {
            // A broken output pipe must not abort restoration.
            let _ = writeln!(
                std::io::stdout(),
                "{}",
                json!({"status":status,"ducked":!mixer.leases.is_empty()})
            );
            previous = status;
        }
        if stop {
            // Retry transient server failures without retaining a helper forever.
            for _ in 0..10 {
                if mixer.leases.is_empty() {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
                if let Ok(streams) = streams() {
                    let _ = mixer.tick(&root, &streams, false, true, set_volume);
                }
            }
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duck_preserves_user_volume_and_restores_without_overwriting_adjustments() -> Result<()> {
        let root = process(std::process::id()).unwrap();
        let mut streams = vec![Stream {
            id: Identity {
                index: 1,
                client: 2,
                serial: "3".into(),
            },
            properties: json!({"application.process.id":root.pid}),
            volume: vec![0.8, 0.4],
            soft: vec![1.0, 1.0],
            muted: false,
        }];
        let mut mixer = Mixer::new();
        let mut writes = Vec::new();
        mixer.tick(&root, &streams, true, true, |_, v| {
            writes.push(v);
            Ok(())
        })?;
        let Change::Attenuate(v) = writes.pop().unwrap() else {
            panic!("no attenuation")
        };
        assert!(same_volume(&v, &[0.8 * DUCK_GAIN, 0.4 * DUCK_GAIN]));
        streams[0].soft = v;
        mixer.tick(&root, &streams, false, true, |_, v| {
            writes.push(v);
            Ok(())
        })?;
        assert!(matches!(writes.pop(), Some(Change::Restore)));
        mixer.tick(&root, &streams, true, true, |_, _| Ok(()))?;
        streams[0].volume = vec![0.3, 0.2];
        mixer.tick(&root, &streams, true, true, |_, _| panic!("overrode user"))?;
        mixer.tick(&root, &streams, false, true, |_, _| {
            panic!("restored stale volume")
        })?;
        streams[0].properties = json!({"application.process.id":1});
        streams[0].id.serial = "replacement".into();
        mixer.tick(&root, &streams, true, true, |_, _| {
            panic!("touched unrelated stream")
        })?;
        Ok(())
    }
    #[test]
    fn native_client_identity_and_channels_are_preserved() -> Result<()> {
        let streams = parse_streams(
            r#"[{"id":4,"info":{"props":{"media.class":"Stream/Output/Audio","client.id":2,"object.serial":50},"params":{"Props":[{"volume":1,"mute":true,"channelVolumes":[0.8,0.4],"softVolumes":[1,1]}]}}},{"id":2,"info":{"props":{"pipewire.sec.pid":1234}}}]"#,
        )?;
        assert_eq!(streams[0].volume, vec![0.8, 0.4]);
        assert_eq!(streams[0].properties["pipewire.sec.pid"], 1234);
        assert!(streams[0].muted);
        let pending = parse_streams(
            r#"[{"id":4,"info":{"props":{"media.class":"Stream/Output/Audio","client.id":2,"object.serial":50},"params":{"Props":[{"mute":false,"channelVolumes":[0.8,0.4],"softVolumes":[]}]}}}]"#,
        )?;
        assert_eq!(pending[0].soft, pending[0].volume);
        Ok(())
    }
}
