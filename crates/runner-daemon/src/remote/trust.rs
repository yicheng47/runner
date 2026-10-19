use chrono::{DateTime, Utc};
use runner_core::remote::{encode_key, fingerprint, PairedDevice};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trust {
    pub version: u8,
    pub devices: Vec<PairedDevice>,
    pub failed_pins: u8,
    pub pin_locked: bool,
}
impl Default for Trust {
    fn default() -> Self {
        Self {
            version: 1,
            devices: Vec::new(),
            failed_pins: 0,
            pin_locked: false,
        }
    }
}
impl Trust {
    pub fn load(root: &Path) -> Result<Self, &'static str> {
        let path = root.join("remote-trust.json");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(_) => return Err("Could not read remote-trust.json"),
        };
        let value = serde_json::from_slice::<Self>(&bytes).ok().filter(|v| {
            v.version == 1 && v.failed_pins <= 5 && (v.failed_pins == 5) == v.pin_locked
        });
        match value {
            Some(value) => Ok(value),
            None => {
                log::warn!(
                    "remote: invalid remote-trust.json; preserving as remote-trust.json.bad"
                );
                std::fs::rename(path, root.join("remote-trust.json.bad"))
                    .map_err(|_| "Could not preserve invalid remote-trust.json")?;
                Ok(Self::default())
            }
        }
    }
    pub fn save(&self, root: &Path) -> Result<(), &'static str> {
        std::fs::create_dir_all(root).map_err(|_| "Could not create remote trust directory")?;
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
            .map_err(|_| "Could not create remote trust file")?;
        serde_json::to_writer(&mut file, self).map_err(|_| "Could not serialize remote trust")?;
        file.flush().map_err(|_| "Could not write remote trust")?;
        file.as_file()
            .sync_all()
            .map_err(|_| "Could not sync remote trust")?;
        file.persist(root.join("remote-trust.json"))
            .map_err(|_| "Could not replace remote trust")?;
        Ok(())
    }
    pub fn record_failed_pin(&mut self) {
        self.failed_pins = (self.failed_pins + 1).min(5);
        self.pin_locked = self.failed_pins == 5;
    }
    pub fn unlock(&mut self) {
        self.failed_pins = 0;
        self.pin_locked = false;
    }
    pub fn record_paired(
        &mut self,
        public_key: &[u8],
        name: String,
        now: DateTime<Utc>,
    ) -> Result<(), &'static str> {
        if self.pin_locked {
            return Err("PIN entry is locked. Unlock it in Settings.");
        }
        if public_key.len() != 32 {
            return Err("Invalid device public key");
        }
        let fingerprint = fingerprint(public_key);
        if let Some(device) = self
            .devices
            .iter_mut()
            .find(|d| d.fingerprint == fingerprint)
        {
            device.name = name;
            device.last_seen_at = now;
        } else {
            self.devices.push(PairedDevice {
                fingerprint,
                name,
                public_key: encode_key(public_key),
                paired_at: now,
                last_seen_at: now,
            });
        }
        self.failed_pins = 0;
        Ok(())
    }
    pub fn revoke(&mut self, fingerprint: &str) -> bool {
        let count = self.devices.len();
        self.devices.retain(|d| d.fingerprint != fingerprint);
        self.devices.len() != count
    }
}

pub fn validate_pin(pin: &str) -> Result<(), &'static str> {
    if pin.len() != 6 || !pin.bytes().all(|c| c.is_ascii_digit()) {
        return Err("The PIN must be exactly six digits.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pin_validation_keeps_leading_zeroes_and_accepts_only_digits() {
        for pin in ["000000", "012345"] {
            assert!(validate_pin(pin).is_ok());
        }
        for pin in [
            "",
            "12345",
            "1234567",
            "0123456789",
            "１２３４５６",
            " 123456",
            "123456\n",
            "12345a",
        ] {
            assert_eq!(
                validate_pin(pin),
                Err("The PIN must be exactly six digits.")
            );
        }
    }
    #[test]
    fn failures_survive_restart_lock_at_five_and_success_resets_the_count() {
        let root = tempfile::tempdir().unwrap();
        let mut trust = Trust::default();
        for count in 1..=5 {
            trust.record_failed_pin();
            trust.save(root.path()).unwrap();
            trust = Trust::load(root.path()).unwrap();
            assert_eq!(trust.failed_pins, count);
            assert_eq!(trust.pin_locked, count == 5);
        }
        let now = Utc::now();
        assert!(trust
            .record_paired(&[3; 32], "Browser".into(), now)
            .is_err());
        trust.record_failed_pin();
        assert_eq!(trust.failed_pins, 5);
        trust.unlock();
        assert_eq!(trust.failed_pins, 0);
        trust.record_failed_pin();
        trust
            .record_paired(&[3; 32], "Browser".into(), now)
            .unwrap();
        assert_eq!(trust.failed_pins, 0);
        trust.save(root.path()).unwrap();
        let mut restored = Trust::load(root.path()).unwrap();
        assert_eq!(restored.devices, trust.devices);
        assert!(restored.revoke(&fingerprint(&[3; 32])));
        restored.save(root.path()).unwrap();
        assert!(Trust::load(root.path()).unwrap().devices.is_empty());
    }
    #[test]
    fn trust_file_is_replaced_privately_and_bad_data_preserved() {
        let root = tempfile::tempdir().unwrap();
        let mut trust = Trust::default();
        trust.save(root.path()).unwrap();
        trust.record_failed_pin();
        trust.save(root.path()).unwrap();
        assert_eq!(Trust::load(root.path()).unwrap().failed_pins, 1);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.path().join("remote-trust.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        for bad in [
            "{",
            r#"{"version":2,"devices":[],"failed_pins":0,"pin_locked":false}"#,
        ] {
            std::fs::write(root.path().join("remote-trust.json"), bad).unwrap();
            assert!(Trust::load(root.path()).unwrap().devices.is_empty());
            assert_eq!(
                std::fs::read_to_string(root.path().join("remote-trust.json.bad")).unwrap(),
                bad
            );
        }
    }
}
