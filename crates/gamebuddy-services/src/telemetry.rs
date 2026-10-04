//! Optional read-only emulator evidence. Profiles are ROM-payload specific.
//! Unknown revisions fail closed; an area/screen tuple is not a named landmark.
use crate::{Store, screen};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    net::{SocketAddr, UdpSocket},
    path::{Path, PathBuf},
    time::Duration,
};

const FAXANADU_US: &str = "52f7e7309f1f53f632912182f3a65f574c949392ab92f55c2494a7e648ef5ccb";
const PROVENANCE: &str =
    "https://github.com/chipx86/faxanadu/tree/eb0b5277be1c43a3bfdeda2a73acbc4c33580f47";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub endpoint: SocketAddr,
    pub rom: PathBuf,
    pub game_pid: u32,
    pub session: String,
    pub payload_sha256: String,
}

pub fn identify(rom: &Path) -> Result<String> {
    let bytes = std::fs::read(rom)?;
    ensure!(
        bytes.len() == 262160 && &bytes[..4] == b"NES\x1a" && bytes[6] & 4 == 0,
        "Adapter requires the verified NES ROM layout without a trainer"
    );
    let hash = format!("{:x}", Sha256::digest(&bytes[16..]));
    ensure!(
        hash == FAXANADU_US,
        "This Faxanadu ROM revision has no verified RAM profile"
    );
    Ok(hash)
}

pub fn attach(store: &Store, rom: &Path, pid: u32, endpoint: SocketAddr) -> Result<Config> {
    ensure!(
        endpoint.ip().is_loopback(),
        "Emulator adapter requires a loopback endpoint"
    );
    let status = screen::status(store)?;
    let session = status["session_id"]
        .as_str()
        .context("Start a game capture session first")?;
    let config = Config {
        endpoint,
        rom: rom.canonicalize()?,
        game_pid: pid,
        session: session.into(),
        payload_sha256: identify(rom)?,
    };
    validate_process(&config)?;
    validate_content(endpoint, &config.rom)?;
    store.put("emulator-adapter", &config)?;
    Ok(config)
}

fn validate_process(config: &Config) -> Result<()> {
    // Binding to the actual launched content prevents a correct memory layout
    // from being applied to another RetroArch process or a different game.
    #[cfg(target_os = "linux")]
    {
        let args = std::fs::read(format!("/proc/{}/cmdline", config.game_pid))?;
        ensure!(
            args.split(|b| *b == 0)
                .filter_map(|b| std::str::from_utf8(b).ok())
                .any(|s| Path::new(s).canonicalize().ok().as_ref() == Some(&config.rom)),
            "Emulator process does not have this ROM loaded"
        );
        let owned: std::collections::HashSet<_> =
            std::fs::read_dir(format!("/proc/{}/fd", config.game_pid))?
                .flatten()
                .filter_map(|e| std::fs::read_link(e.path()).ok())
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
        let mut bound = false;
        for table in ["udp", "udp6"] {
            let sockets =
                std::fs::read_to_string(format!("/proc/{}/net/{table}", config.game_pid))?;
            for line in sockets.lines().skip(1) {
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields.len() > 9
                    && fields[1]
                        .rsplit(':')
                        .next()
                        .and_then(|p| u16::from_str_radix(p, 16).ok())
                        == Some(config.endpoint.port())
                    && owned.contains(&format!("socket:[{}]", fields[9]))
                {
                    bound = true;
                }
            }
        }
        ensure!(
            bound,
            "Emulator endpoint does not belong to the verified game process"
        );
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Automatic emulator process binding is currently implemented on Linux only")
    }
}

fn validate_content(endpoint: SocketAddr, rom: &Path) -> Result<()> {
    let socket = UdpSocket::bind(if endpoint.is_ipv4() {
        "127.0.0.1:0"
    } else {
        "[::1]:0"
    })?;
    socket.set_read_timeout(Some(Duration::from_millis(350)))?;
    socket.connect(endpoint)?;
    socket.send(b"GET_STATUS\n")?;
    let mut buffer = [0u8; 4096];
    let count = socket.recv(&mut buffer)?;
    let reply = std::str::from_utf8(&buffer[..count])?.trim();
    let content = reply
        .strip_prefix("GET_STATUS PLAYING ")
        .and_then(|s| s.split_once(','))
        .map(|(_, s)| s);
    ensure!(
        content == rom.file_stem().and_then(|s| s.to_str()),
        "Emulator endpoint is not playing the verified content"
    );
    Ok(())
}

/// Strict response validation; never accept partial memory as zero-filled RAM.
pub fn parse_ram(reply: &str, address: u32, length: usize) -> Result<Vec<u8>> {
    let mut words = reply.split_whitespace();
    ensure!(
        words.next() == Some("READ_CORE_MEMORY"),
        "Unexpected emulator response"
    );
    ensure!(
        u32::from_str_radix(words.next().context("Missing memory address")?, 16)? == address,
        "Wrong memory address in emulator response"
    );
    let bytes = words
        .map(|word| u8::from_str_radix(word, 16))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(bytes.len() == length, "Incomplete emulator memory response");
    Ok(bytes)
}

pub fn read_ram(endpoint: SocketAddr) -> Result<Vec<u8>> {
    ensure!(
        endpoint.ip().is_loopback(),
        "Emulator endpoint must be local"
    );
    let socket = UdpSocket::bind(if endpoint.is_ipv4() {
        "127.0.0.1:0"
    } else {
        "[::1]:0"
    })?;
    socket.set_read_timeout(Some(Duration::from_millis(350)))?;
    socket.connect(endpoint)?;
    socket.send(b"READ_CORE_MEMORY 00000000 2048\n")?;
    let mut buffer = [0u8; 16384];
    let count = socket.recv(&mut buffer)?;
    parse_ram(std::str::from_utf8(&buffer[..count])?, 0, 2048)
}

pub fn read_state(rom: &Path, endpoint: SocketAddr) -> Result<Value> {
    identify(rom)?;
    ensure!(
        endpoint.ip().is_loopback(),
        "Emulator endpoint must be local"
    );
    validate_content(endpoint, rom)?;
    decode_faxanadu(&read_ram(endpoint)?)
}

pub fn decode_faxanadu(ram: &[u8]) -> Result<Value> {
    ensure!(
        ram.len() == 2048,
        "Faxanadu needs a complete 2 KiB RAM snapshot"
    );
    ensure!(ram[0x24] <= 7, "Invalid area number for this ROM profile");
    let inventories = [
        ("weapons", 0x39d, 4, 0x3c2),
        ("armor", 0x3a1, 4, 0x3c3),
        ("shields", 0x3a5, 4, 0x3c4),
        ("magic", 0x3a9, 4, 0x3c5),
        ("items", 0x3ad, 16, 0x3c6),
    ];
    let mut inventory = serde_json::Map::new();
    for (name, start, capacity, count_at) in inventories {
        let count = ram[count_at] as usize;
        ensure!(count <= capacity, "Invalid inventory count");
        inventory.insert(name.into(), json!(&ram[start..start + count]));
    }
    Ok(
        json!({"profile":"faxanadu-us-v1","gameplay_verified":false,"interpretation":"Raw memory snapshot; area/screen may be stale during menus or transitions. Corroborate with the screen.","area":ram[0x24],"screen":ram[0x63],"loading_screen":ram[0x64],"room_key":format!("faxanadu-{:02x}-{:02x}",ram[0x24],ram[0x63]),"raw_player_position":{"x":ram[0x9e],"y":ram[0xa1],"coordinate_space":"NES PPU sprite anchor; not atlas coordinates"},"hp_integer":ram[0x431],"mp":ram[0x39a],"dead":ram[0x438]!=0,"transitioning":ram[0x3db]!=0 || ram[0x3dc]!=0 || ram[0x430]!=0,"inventory":inventory,"quest_bits":ram[0x42d],"special_item_bits":ram[0x42c],"provenance":PROVENANCE}),
    )
}

pub fn observe(store: &Store, snapshot: &screen::Snapshot, epoch: u64) -> Result<Option<Value>> {
    let Some(config) = store.get::<Config>("emulator-adapter")? else {
        return Ok(None);
    };
    ensure!(
        config.session == snapshot.ticket.session,
        "Emulator adapter belongs to an earlier session"
    );
    validate_process(&config)?;
    ensure!(
        config.payload_sha256 == FAXANADU_US,
        "Unknown emulator profile"
    );
    validate_content(config.endpoint, &config.rom)?;
    let state = decode_faxanadu(&read_ram(config.endpoint)?)?;
    let before: Option<Value> = store.get("emulator-observation")?;
    let mut events = vec![];
    if let Some(previous) = before.filter(|v| {
        v["session"] == config.session
            && v["epoch"].as_u64() == Some(epoch)
            && snapshot
                .captured_at_ms
                .saturating_sub(v["at_ms"].as_u64().unwrap_or(0))
                < 5000
    }) {
        for key in [
            "room_key",
            "inventory",
            "dead",
            "quest_bits",
            "special_item_bits",
        ] {
            if previous["state"][key] != state[key] {
                events.push(json!({"field":key,"before":previous["state"][key],"after":state[key],"kind":"observed_state_change"}));
            }
        }
    }
    ensure!(
        screen::ticket(store, &config.session)? == snapshot.ticket,
        "Capture changed during emulator read"
    );
    let result = json!({"session":config.session,"epoch":epoch,"at_ms":snapshot.captured_at_ms,"state":state,"events":events,"evidence_kind":"emulator_memory","objective_completion":false});
    store.put("emulator-observation", &result)?;
    Ok(Some(result))
}

pub fn context(store: &Store, session: &Value, epoch: u64) -> Result<Option<Value>> {
    Ok(store.get::<Value>("emulator-observation")?.filter(|v| {
        v["session"] == session["session_id"]
            && v["epoch"].as_u64() == Some(epoch)
            && screen::now_ms().saturating_sub(v["at_ms"].as_u64().unwrap_or(0)) < 5000
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_wrong_address_partial_and_error_replies() {
        assert!(parse_ram("READ_CORE_MEMORY 10 01 02", 0, 2).is_err());
        assert!(parse_ram("READ_CORE_MEMORY 0 01", 0, 2).is_err());
        assert!(parse_ram("READ_CORE_MEMORY 0 -1", 0, 2).is_err());
        assert_eq!(
            parse_ram("READ_CORE_MEMORY 0 01 ff\n", 0, 2).unwrap(),
            [1, 255]
        );
    }
    #[test]
    fn memory_distinguishes_area_screen_inventory_and_death() {
        let mut ram = vec![0; 2048];
        ram[0x24] = 3;
        ram[0x63] = 9;
        ram[0x9e] = 100;
        ram[0xa1] = 160;
        ram[0x3c2] = 1;
        ram[0x39d] = 2;
        ram[0x438] = 1;
        let state = decode_faxanadu(&ram).unwrap();
        assert_eq!(state["room_key"], "faxanadu-03-09");
        assert_eq!(state["inventory"]["weapons"], json!([2]));
        assert_eq!(state["dead"], true);
        ram[0x3c2] = 5;
        assert!(decode_faxanadu(&ram).is_err());
    }
}
