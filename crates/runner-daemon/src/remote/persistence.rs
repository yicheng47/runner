use chrono::{DateTime, Utc};
use runner_core::remote::{canonical_origin, valid_relay_url, RemoteAccount, StopReason};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub version: u8,
    pub origin: String,
    pub host_id: String,
    pub relay_url: String,
    pub account: RemoteAccount,
    pub host_name: String,
    pub fingerprint: String,
    pub activated: bool,
    pub claim_expires_at: DateTime<Utc>,
    pub paused: bool,
    pub stopped: Option<StopReason>,
}
impl Saved {
    pub fn load(root: &Path) -> Result<Option<Self>, &'static str> {
        let path = root.join("remote.json");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Could not read remote.json"),
        };
        let value = serde_json::from_slice::<Self>(&bytes).ok().filter(|v| {
            v.version == 1
                && canonical_origin(&v.origin).is_ok_and(|o| o == v.origin)
                && valid_relay_url(&v.origin, &v.relay_url)
        });
        if value.is_none() {
            log::warn!("remote: invalid remote.json; preserving as remote.json.bad");
            std::fs::rename(path, root.join("remote.json.bad"))
                .map_err(|_| "Could not preserve invalid remote.json")?;
        }
        Ok(value)
    }
    pub fn save(&self, root: &Path) -> Result<(), &'static str> {
        std::fs::create_dir_all(root).map_err(|_| "Could not create remote state directory")?;
        let builder = tempfile::Builder::new();
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::PermissionsExt;
            let mut builder = builder;
            builder.permissions(std::fs::Permissions::from_mode(0o600));
            builder
        };
        let mut file = builder
            .tempfile_in(root)
            .map_err(|_| "Could not create remote state file")?;
        serde_json::to_writer(&mut file, self).map_err(|_| "Could not serialize remote state")?;
        file.flush().map_err(|_| "Could not write remote state")?;
        file.as_file()
            .sync_all()
            .map_err(|_| "Could not sync remote state")?;
        file.persist(root.join("remote.json"))
            .map_err(|_| "Could not replace remote state")?;
        Ok(())
    }
    pub fn delete(root: &Path) -> Result<(), &'static str> {
        match std::fs::remove_file(root.join("remote.json")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Could not delete remote.json"),
        }
    }
}
