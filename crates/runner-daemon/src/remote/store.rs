use std::path::Path;
#[cfg(not(any(target_os = "macos", windows)))]
use std::path::PathBuf;

pub type StoreResult<T> = Result<T, &'static str>;
pub trait SecretStore: Send + Sync {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>>;
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()>;
    fn delete(&self, origin: &str) -> StoreResult<()>;
}

pub struct OsStore {
    namespace: String,
    #[cfg(not(any(target_os = "macos", windows)))]
    root: PathBuf,
}
impl OsStore {
    pub fn new(root: &Path, debug: bool) -> Self {
        Self::named(
            root,
            &format!("{}.remote", runner_core::app_paths::app_identifier(debug)),
        )
    }
    fn named(_root: &Path, namespace: &str) -> Self {
        Self {
            namespace: namespace.into(),
            #[cfg(not(any(target_os = "macos", windows)))]
            root: _root.join("remote-secrets.json"),
        }
    }
}
#[cfg(target_os = "macos")]
impl OsStore {
    fn options(&self, origin: &str) -> security_framework::passwords::PasswordOptions {
        let mut options = security_framework::passwords::PasswordOptions::new_generic_password(
            &self.namespace,
            origin,
        );
        options.set_access_synchronized(Some(false));
        options.set_label("Runner remote access");
        options
    }
}
#[cfg(target_os = "macos")]
impl SecretStore for OsStore {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>> {
        match security_framework::passwords::generic_password(self.options(origin)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.code() == -25300 => Ok(None),
            Err(_) => Err("Keychain read failed"),
        }
    }
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()> {
        security_framework::passwords::set_generic_password_options(value, self.options(origin))
            .map_err(|_| "Keychain write failed")
    }
    fn delete(&self, origin: &str) -> StoreResult<()> {
        match security_framework::passwords::delete_generic_password_options(self.options(origin)) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == -25300 => Ok(()),
            Err(_) => Err("Keychain deletion failed"),
        }
    }
}
#[cfg(windows)]
impl OsStore {
    fn target(&self, origin: &str) -> Vec<u16> {
        format!("{}/{}", self.namespace, origin)
            .encode_utf16()
            .chain(Some(0))
            .collect()
    }
}
#[cfg(windows)]
impl SecretStore for OsStore {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>> {
        use windows_sys::Win32::{
            Foundation::{GetLastError, ERROR_NOT_FOUND},
            Security::Credentials::*,
        };
        let target = self.target(origin);
        let mut item = std::ptr::null_mut();
        unsafe {
            if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut item) == 0 {
                return if GetLastError() == ERROR_NOT_FOUND {
                    Ok(None)
                } else {
                    Err("Credential Manager read failed")
                };
            }
            let bytes = if (*item).CredentialBlobSize == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(
                    (*item).CredentialBlob,
                    (*item).CredentialBlobSize as usize,
                )
                .to_vec()
            };
            CredFree(item.cast());
            Ok(Some(bytes))
        }
    }
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()> {
        use windows_sys::Win32::Security::Credentials::*;
        let mut target = self.target(origin);
        let item = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: value.len() as u32,
            CredentialBlob: value.as_ptr().cast_mut(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            ..Default::default()
        };
        if unsafe { CredWriteW(&item, 0) } == 0 {
            Err("Credential Manager write failed")
        } else {
            Ok(())
        }
    }
    fn delete(&self, origin: &str) -> StoreResult<()> {
        use windows_sys::Win32::{
            Foundation::{GetLastError, ERROR_NOT_FOUND},
            Security::Credentials::*,
        };
        if unsafe { CredDeleteW(self.target(origin).as_ptr(), CRED_TYPE_GENERIC, 0) } != 0
            || unsafe { GetLastError() } == ERROR_NOT_FOUND
        {
            Ok(())
        } else {
            Err("Credential Manager deletion failed")
        }
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
impl OsStore {
    fn path(&self, _origin: &str) -> PathBuf {
        self.root.clone()
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
impl SecretStore for OsStore {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>> {
        use std::os::unix::fs::MetadataExt;
        let path = self.path(origin);
        let file = match std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("Secret file read failed"),
        };
        let meta = file.metadata().map_err(|_| "Secret file metadata failed")?;
        if meta.mode() & 0o077 != 0 || meta.uid() != unsafe { libc::geteuid() } {
            return Err("Secret file must be owned by this user with mode 0600");
        }
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(4096)
            .read_to_end(&mut bytes)
            .map_err(|_| "Secret file read failed")?;
        Ok(Some(bytes))
    }
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()> {
        use std::io::Write;
        std::fs::create_dir_all(self.root.parent().ok_or("Secret file has no parent")?)
            .map_err(|_| "Secret directory creation failed")?;
        let mut file = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o600))
            .tempfile_in(self.root.parent().ok_or("Secret file has no parent")?)
            .map_err(|_| "Secret file creation failed")?;
        file.write_all(value)
            .map_err(|_| "Secret file write failed")?;
        file.as_file()
            .sync_all()
            .map_err(|_| "Secret file sync failed")?;
        file.persist(self.path(origin))
            .map_err(|_| "Secret file replacement failed")?;
        Ok(())
    }
    fn delete(&self, origin: &str) -> StoreResult<()> {
        match std::fs::remove_file(self.path(origin)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Secret file deletion failed"),
        }
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn os_store_round_trip() {
        let root = tempfile::tempdir().unwrap();
        let namespace = format!("com.wycstudios.runner.remote.test.{}", uuid::Uuid::new_v4());
        let store = OsStore::named(root.path(), &namespace);
        let origin = "https://store-test.invalid";
        struct Cleanup<'a>(&'a OsStore, &'a str);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.delete(self.1);
            }
        }
        let _cleanup = Cleanup(&store, origin);
        assert!(store.get(origin).unwrap().is_none());
        let value = br#"{"v":1,"key":"test-key","credential":"test-only-credential"}"#;
        store
            .put(origin, value)
            .expect("OS secret store must be reachable; do not skip this check");
        assert_eq!(
            store.get(origin).unwrap().as_deref(),
            Some(value.as_slice())
        );
        store.put(origin, b"replacement").unwrap();
        assert_eq!(store.get(origin).unwrap(), Some(b"replacement".to_vec()));
        store.delete(origin).unwrap();
        assert!(store.get(origin).unwrap().is_none());
    }
    #[test]
    fn dev_store_names_are_separate() {
        assert_ne!(
            OsStore::new(Path::new("unused"), true).namespace,
            OsStore::new(Path::new("unused"), false).namespace
        );
    }
}

#[derive(Default)]
pub struct MemoryStore(std::sync::Mutex<std::collections::HashMap<String, Vec<u8>>>);
impl SecretStore for MemoryStore {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>> {
        Ok(self
            .0
            .lock()
            .map_err(|_| "Secret store lock failed")?
            .get(origin)
            .cloned())
    }
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()> {
        self.0
            .lock()
            .map_err(|_| "Secret store lock failed")?
            .insert(origin.into(), value.into());
        Ok(())
    }
    fn delete(&self, origin: &str) -> StoreResult<()> {
        self.0
            .lock()
            .map_err(|_| "Secret store lock failed")?
            .remove(origin);
        Ok(())
    }
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Secrets {
    pub v: u8,
    pub key: runner_core::remote::Secret,
    pub credential: runner_core::remote::Secret,
}
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}
