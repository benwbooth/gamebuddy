//! Session-local commands shared by keyboard, launcher and controller adapters.
use anyhow::{Result, bail};
use std::{
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Action {
    Toggle = 1,
    ToggleVisibility = 2,
    Show = 3,
    Hide = 4,
    Focus = 5,
    Release = 6,
}
impl Action {
    pub fn parse(value: &str) -> Result<Self> {
        Ok(match value.trim() {
            "toggle" => Self::Toggle,
            "toggle-visibility" => Self::ToggleVisibility,
            "show" => Self::Show,
            "hide" => Self::Hide,
            "focus" => Self::Focus,
            "release" => Self::Release,
            _ => bail!(
                "Unknown overlay action; use toggle, toggle-visibility, show, hide, focus or release"
            ),
        })
    }
}
/// The socket lives in a private per-launch directory; no global listener.
pub struct Endpoint {
    socket: UnixDatagram,
    path: PathBuf,
}
impl Endpoint {
    pub fn bind(path: PathBuf) -> Result<Self> {
        let socket = UnixDatagram::bind(&path)?;
        socket.set_nonblocking(true)?;
        Ok(Self { socket, path })
    }
    pub fn poll(&self) -> Result<Option<Action>> {
        let mut bytes = [0_u8; 64];
        match self.socket.recv(&mut bytes) {
            Ok(length) => Action::parse(std::str::from_utf8(&bytes[..length])?).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
pub fn send(path: &Path, action: &str) -> Result<()> {
    Action::parse(action)?;
    UnixDatagram::unbound()?.send_to(action.as_bytes(), path)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn commands_are_scoped_to_the_session_and_socket_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("control.sock");
        let endpoint = Endpoint::bind(path.clone()).unwrap();
        assert_eq!(endpoint.poll().unwrap(), None);
        send(&path, "hide").unwrap();
        assert_eq!(endpoint.poll().unwrap(), Some(Action::Hide));
        assert!(send(&path, "arbitrary command").is_err());
        assert!(Endpoint::bind(path.clone()).is_err());
        drop(endpoint);
        assert!(!path.exists());
    }
}
