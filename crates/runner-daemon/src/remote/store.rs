use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

pub type StoreResult<T> = Result<T, &'static str>;
pub trait SecretStore: Send + Sync {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>>;
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()>;
    fn delete(&self, origin: &str) -> StoreResult<()>;
}

pub struct OsStore {
    #[cfg(any(target_os = "macos", windows, test))]
    namespace: String,
    #[cfg(unix)]
    file: Option<FileStore>,
}
impl OsStore {
    pub fn new(root: &Path, debug: bool) -> Self {
        let store = Self::named(
            root,
            &format!("{}.remote", runner_core::app_paths::app_identifier(debug)),
        );
        #[cfg(target_os = "macos")]
        let store = Self {
            file: debug.then(|| FileStore { root: root.into() }),
            ..store
        };
        store
    }
    fn named(_root: &Path, _namespace: &str) -> Self {
        Self {
            #[cfg(any(target_os = "macos", windows, test))]
            namespace: _namespace.into(),
            #[cfg(unix)]
            file: if cfg!(target_os = "macos") {
                None
            } else {
                Some(FileStore { root: _root.into() })
            },
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
        if let Some(file) = &self.file {
            return file.get(origin);
        }
        match security_framework::passwords::generic_password(self.options(origin)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.code() == -25300 => Ok(None),
            Err(_) => Err("Keychain read failed"),
        }
    }
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()> {
        if let Some(file) = &self.file {
            return file.put(origin, value);
        }
        security_framework::passwords::set_generic_password_options(value, self.options(origin))
            .map_err(|_| "Keychain write failed")
    }
    fn delete(&self, origin: &str) -> StoreResult<()> {
        if let Some(file) = &self.file {
            return file.delete(origin);
        }
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

#[cfg(unix)]
struct FileStore {
    root: PathBuf,
}
#[cfg(unix)]
impl FileStore {
    fn validate_root(&self) -> StoreResult<bool> {
        use std::os::unix::fs::MetadataExt;
        let meta = match std::fs::symlink_metadata(&self.root) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err("Secret directory metadata failed"),
        };
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
            return Err("Secret directory must be owned by this user and not writable by others");
        }
        Ok(true)
    }

    fn path(&self, name: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        let filename = match name {
            IDENTITY_NAME => "remote-identity.json".into(),
            CLIENT_KEYS_NAME => "remote-client-keys.json".into(),
            _ => format!(
                "remote-credential-{:x}.json",
                Sha256::digest(name.as_bytes())
            ),
        };
        self.root.join(filename)
    }
}
#[cfg(unix)]
impl SecretStore for FileStore {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>> {
        use std::os::unix::fs::MetadataExt;
        if !self.validate_root()? {
            return Ok(None);
        }
        let path = self.path(origin);
        let mut file = match std::fs::OpenOptions::new()
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
        file.read_to_end(&mut bytes)
            .map_err(|_| "Secret file read failed")?;
        Ok(Some(bytes))
    }
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()> {
        use std::io::Write;
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.root)
            .map_err(|_| "Secret directory creation failed")?;
        self.validate_root()?;
        let mut file = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o600))
            .tempfile_in(&self.root)
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
        if !self.validate_root()? {
            return Ok(());
        }
        match std::fs::remove_file(self.path(origin)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err("Secret file deletion failed"),
        }
    }
}
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[cfg(not(any(target_os = "macos", windows)))]
impl SecretStore for OsStore {
    fn get(&self, origin: &str) -> StoreResult<Option<Vec<u8>>> {
        self.file.as_ref().unwrap().get(origin)
    }
    fn put(&self, origin: &str, value: &[u8]) -> StoreResult<()> {
        self.file.as_ref().unwrap().put(origin, value)
    }
    fn delete(&self, origin: &str) -> StoreResult<()> {
        self.file.as_ref().unwrap().delete(origin)
    }
}

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
    fn identity_os_store_round_trip_and_enrollment_deletion_are_independent() {
        let root = tempfile::tempdir().unwrap();
        let namespace = format!("com.wycstudios.runner.remote.test.{}", uuid::Uuid::new_v4());
        let store = OsStore::named(root.path(), &namespace);
        struct Cleanup<'a>(&'a OsStore);
        impl Drop for Cleanup<'_> {
            fn drop(&mut self) {
                let _ = self.0.delete(IDENTITY_NAME);
                let _ = self.0.delete("https://identity-test.invalid");
            }
        }
        let _cleanup = Cleanup(&store);
        let mut identity = Identity::generate().unwrap();
        identity.pin = Some(runner_core::remote::Secret("001234".into()));
        identity.save(&store).unwrap();
        store
            .put(
                "https://identity-test.invalid",
                br#"{"v":2,"credential":"test"}"#,
            )
            .unwrap();
        store.delete("https://identity-test.invalid").unwrap();
        let restored = Identity::load(&store).unwrap().unwrap();
        assert_eq!(
            restored.host_key().unwrap().public,
            identity.host_key().unwrap().public
        );
        assert_eq!(restored.pin.as_ref().unwrap().0, "001234");
        assert!(!format!("{restored:?}").contains("001234"));
        assert!(!format!("{restored:?}").contains(&identity.key.0));
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.path().join("remote-identity.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert!(!root.path().join("remote-secrets.json").exists());
        }
    }
    #[test]
    fn upgrade_adopts_legacy_key_only_without_an_identity_and_rewrites_enrollment() {
        for existing in [false, true] {
            let store = MemoryStore::default();
            let legacy = Identity::generate().unwrap();
            let expected = if existing {
                Identity::generate().unwrap()
            } else {
                legacy.clone()
            };
            if existing {
                expected.save(&store).unwrap();
            }
            let origin = "https://upgrade-test.invalid";
            store
                .put(
                    origin,
                    &serde_json::to_vec(&Secrets {
                        v: 1,
                        key: Some(legacy.key),
                        credential: runner_core::remote::Secret(format!("rhc_{}", "x".repeat(43))),
                    })
                    .unwrap(),
                )
                .unwrap();
            let (identity, enrollment) = restore(&store, Some(origin)).unwrap();
            assert_eq!(
                identity.unwrap().host_key().unwrap().public,
                expected.host_key().unwrap().public
            );
            assert_eq!(enrollment.unwrap().v, 2);
            let value: serde_json::Value =
                serde_json::from_slice(&store.get(origin).unwrap().unwrap()).unwrap();
            assert!(value.get("key").is_none());
            let (again, _) = restore(&store, Some(origin)).unwrap();
            assert_eq!(again.unwrap().key.0, expected.key.0);
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<runner_core::remote::Secret>,
    pub credential: runner_core::remote::Secret,
}
impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

pub const IDENTITY_NAME: &str = "host-identity";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Identity {
    pub v: u8,
    pub key: runner_core::remote::Secret,
    pub pin: Option<runner_core::remote::Secret>,
}
impl Identity {
    pub fn generate() -> StoreResult<Self> {
        let key = runner_core::remote::HostKey::generate()
            .map_err(|_| "Could not generate a host key")?;
        Ok(Self {
            v: 1,
            key: runner_core::remote::Secret(runner_core::remote::encode_key(&key.private)),
            pin: None,
        })
    }
    pub fn host_key(&self) -> StoreResult<runner_core::remote::HostKey> {
        runner_core::remote::HostKey::from_private(&self.key.0)
    }
    pub fn load(store: &dyn SecretStore) -> StoreResult<Option<Self>> {
        store
            .get(IDENTITY_NAME)?
            .map(|bytes| {
                let identity: Self =
                    serde_json::from_slice(&bytes).map_err(|_| "Invalid host identity")?;
                if identity.v != 1 {
                    return Err("Invalid host identity version");
                }
                identity.host_key()?;
                Ok(identity)
            })
            .transpose()
    }
    pub fn save(&self, store: &dyn SecretStore) -> StoreResult<()> {
        let bytes = serde_json::to_vec(self).map_err(|_| "Could not encode host identity")?;
        store.put(IDENTITY_NAME, &bytes)
    }
}

pub fn restore(
    store: &dyn SecretStore,
    origin: Option<&str>,
) -> StoreResult<(Option<Identity>, Option<Secrets>)> {
    let mut identity = Identity::load(store)?;
    let mut secrets = match origin {
        Some(origin) => store
            .get(origin)?
            .and_then(|bytes| serde_json::from_slice::<Secrets>(&bytes).ok())
            .filter(|s| {
                matches!(s.v, 1 | 2) && runner_core::remote::valid_credential(&s.credential.0)
            }),
        None => None,
    };
    if let Some(secrets) = &mut secrets {
        if secrets.v == 1 {
            if identity.is_none() {
                let key = secrets.key.clone().ok_or("Missing legacy host key")?;
                runner_core::remote::HostKey::from_private(&key.0)?;
                let adopted = Identity {
                    v: 1,
                    key,
                    pin: None,
                };
                adopted.save(store)?;
                identity = Some(adopted);
            }
            secrets.v = 2;
            secrets.key = None;
            let bytes =
                serde_json::to_vec(secrets).map_err(|_| "Could not encode remote secrets")?;
            store.put(origin.ok_or("Missing enrollment origin")?, &bytes)?;
        }
    }
    if identity.is_none() {
        secrets = None;
    }
    Ok((identity, secrets))
}

pub const CLIENT_KEYS_NAME: &str = "client-keys";
const CLIENT_KEYS_CLEANUP_NAME: &str = "client-keys-cleanup";
const CLIENT_KEYS_ITEM_LIMIT: usize = 2560;
const CLIENT_KEYS_SHARDS_PREFIX: &[u8] = b"RUNNER-CLIENT-KEYS:";

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct ClientKeyShards {
    generation: String,
    count: usize,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ClientKeyCleanup {
    previous: Option<ClientKeyShards>,
    pending: Option<ClientKeyShards>,
}
impl ClientKeyCleanup {
    fn save(&self, store: &dyn SecretStore) -> StoreResult<()> {
        let bytes = serde_json::to_vec(self).map_err(|_| "Could not encode client key cleanup")?;
        if bytes.len() > CLIENT_KEYS_ITEM_LIMIT {
            return Err("Client key cleanup record is too large");
        }
        store.put(CLIENT_KEYS_CLEANUP_NAME, &bytes)
    }
    fn recover(
        store: &dyn SecretStore,
        keep: Option<&ClientKeyShards>,
        reset: bool,
    ) -> StoreResult<()> {
        let Some(bytes) = store.get(CLIENT_KEYS_CLEANUP_NAME)? else {
            return Ok(());
        };
        let cleanup: Self = match serde_json::from_slice(&bytes) {
            Ok(cleanup) => cleanup,
            Err(_) if reset => return store.delete(CLIENT_KEYS_CLEANUP_NAME),
            Err(_) => return Err("Invalid client key cleanup"),
        };
        for shards in [cleanup.previous, cleanup.pending].into_iter().flatten() {
            if keep.is_none_or(|keep| keep.generation != shards.generation) {
                shards.delete(store)?;
            }
        }
        store.delete(CLIENT_KEYS_CLEANUP_NAME)
    }
}
impl ClientKeyShards {
    fn decode(bytes: &[u8]) -> StoreResult<Option<Self>> {
        bytes
            .strip_prefix(CLIENT_KEYS_SHARDS_PREFIX)
            .map(|bytes| serde_json::from_slice(bytes).map_err(|_| "Invalid client key shards"))
            .transpose()
    }
    fn name(&self, index: usize) -> String {
        format!("{CLIENT_KEYS_NAME}/{}/{index}", self.generation)
    }
    fn delete(&self, store: &dyn SecretStore) -> StoreResult<()> {
        for index in 0..self.count {
            store.delete(&self.name(index))?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ClientKey {
    pub private: runner_core::remote::Secret,
    pub public: Vec<u8>,
    pub name: String,
    pub paired_at: chrono::DateTime<chrono::Utc>,
}
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ClientKeys(pub std::collections::BTreeMap<String, ClientKey>);
impl ClientKeys {
    pub fn load(store: &dyn SecretStore) -> StoreResult<Self> {
        let Some(bytes) = store.get(CLIENT_KEYS_NAME)? else {
            return Ok(Self::default());
        };
        let bytes = if let Some(shards) = ClientKeyShards::decode(&bytes)? {
            let mut bytes = Vec::new();
            for index in 0..shards.count {
                bytes.extend(
                    store
                        .get(&shards.name(index))?
                        .ok_or("Missing client key shard")?,
                );
            }
            bytes
        } else {
            bytes
        };
        let keys: Self = serde_json::from_slice(&bytes).map_err(|_| "Invalid client keys")?;
        for (fingerprint, key) in &keys.0 {
            runner_core::remote::HostKey::from_private(&key.private.0)?;
            if key.public.len() != 32
                || runner_core::remote::fingerprint(&key.public) != *fingerprint
            {
                return Err("Invalid pinned computer key");
            }
        }
        Ok(keys)
    }
    pub fn save(&self, store: &dyn SecretStore) -> StoreResult<()> {
        self.save_inner(store, false)
    }
    pub fn reset(store: &dyn SecretStore) -> StoreResult<()> {
        Self::default().save_inner(store, true)
    }
    fn save_inner(&self, store: &dyn SecretStore, reset: bool) -> StoreResult<()> {
        let previous = store
            .get(CLIENT_KEYS_NAME)?
            .map(|bytes| ClientKeyShards::decode(&bytes))
            .transpose();
        let previous = match previous {
            Ok(previous) => previous.flatten(),
            Err(_) if reset => None,
            Err(error) => return Err(error),
        };
        if self.0.is_empty() {
            ClientKeyCleanup::recover(store, None, reset)?;
            if let Some(previous) = previous {
                previous.delete(store)?;
            }
            return store.delete(CLIENT_KEYS_NAME);
        }
        ClientKeyCleanup::recover(store, previous.as_ref(), false)?;
        let bytes = serde_json::to_vec(self).map_err(|_| "Could not encode client keys")?;
        let shards = (bytes.len() > CLIENT_KEYS_ITEM_LIMIT).then(|| ClientKeyShards {
            generation: uuid::Uuid::new_v4().to_string(),
            count: bytes.len().div_ceil(CLIENT_KEYS_ITEM_LIMIT),
        });
        if previous.is_some() || shards.is_some() {
            ClientKeyCleanup {
                previous,
                pending: shards.clone(),
            }
            .save(store)?;
        }
        if let Some(shards) = &shards {
            for (index, bytes) in bytes.chunks(CLIENT_KEYS_ITEM_LIMIT).enumerate() {
                store.put(&shards.name(index), bytes)?;
            }
            let mut manifest = CLIENT_KEYS_SHARDS_PREFIX.to_vec();
            manifest.extend(
                serde_json::to_vec(shards).map_err(|_| "Could not encode client key shards")?,
            );
            store.put(CLIENT_KEYS_NAME, &manifest)?;
        } else {
            store.put(CLIENT_KEYS_NAME, &bytes)?;
        }
        // Publication committed the operation; the journal preserves deferred cleanup.
        let _ = ClientKeyCleanup::recover(store, shards.as_ref(), false);
        Ok(())
    }
}

#[cfg(test)]
mod client_key_tests {
    use super::*;
    #[derive(Default)]
    struct LimitedStore {
        memory: MemoryStore,
        fail_manifest: std::sync::atomic::AtomicBool,
    }
    impl SecretStore for LimitedStore {
        fn get(&self, name: &str) -> StoreResult<Option<Vec<u8>>> {
            self.memory.get(name)
        }
        fn put(&self, name: &str, bytes: &[u8]) -> StoreResult<()> {
            assert!(bytes.len() <= 2560);
            if name == CLIENT_KEYS_NAME
                && self
                    .fail_manifest
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Err("test write failure");
            }
            self.memory.put(name, bytes)
        }
        fn delete(&self, name: &str) -> StoreResult<()> {
            self.memory.delete(name)
        }
    }
    #[test]
    fn pairing_growth_shrink_and_delete_fit_windows_credential_items() {
        let store = LimitedStore::default();
        let mut keys = ClientKeys::default();
        for index in 0..100 {
            let key = runner_core::remote::HostKey::generate().unwrap();
            keys.0.insert(
                runner_core::remote::fingerprint(&key.public),
                ClientKey {
                    private: runner_core::remote::Secret(runner_core::remote::encode_key(
                        &key.private,
                    )),
                    public: key.public.to_vec(),
                    name: format!("Host {index} {}", "名".repeat(20)),
                    paired_at: chrono::Utc::now(),
                },
            );
            keys.save(&store).unwrap();
            assert_eq!(ClientKeys::load(&store).unwrap().0.len(), index + 1);
            let count = store.memory.0.lock().unwrap().len();
            assert!(count <= 1 + serde_json::to_vec(&keys).unwrap().len().div_ceil(2560));
        }
        let saved = serde_json::to_vec(&keys).unwrap();
        keys.0.pop_first();
        store
            .fail_manifest
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(keys.save(&store).is_err());
        assert_eq!(
            serde_json::to_vec(&ClientKeys::load(&store).unwrap()).unwrap(),
            saved
        );
        store
            .fail_manifest
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let one = keys.0.pop_first().unwrap();
        keys.0.clear();
        keys.0.insert(one.0, one.1);
        keys.save(&store).unwrap();
        assert_eq!(store.memory.0.lock().unwrap().len(), 1);
        assert_eq!(ClientKeys::load(&store).unwrap().0.len(), 1);
        keys.0.clear();
        keys.save(&store).unwrap();
        assert!(store.memory.0.lock().unwrap().is_empty());
        assert!(ClientKeys::load(&store).unwrap().0.is_empty());
        #[cfg(windows)]
        assert_eq!(
            CLIENT_KEYS_ITEM_LIMIT,
            windows_sys::Win32::Security::Credentials::CRED_MAX_CREDENTIAL_BLOB_SIZE as usize
        );
    }
}

#[cfg(all(test, unix))]
mod file_tests {
    use super::*;
    #[test]
    fn file_store_rejects_writable_or_symlinked_roots_without_changing_existing_modes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("private");
        let store = FileStore { root: path.clone() };
        assert!(store.get(IDENTITY_NAME).unwrap().is_none());
        store.put(IDENTITY_NAME, b"fixture").unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o770)).unwrap();
        assert!(store.get(IDENTITY_NAME).is_err());
        assert!(store.put(IDENTITY_NAME, b"replacement").is_err());
        assert!(store.delete(IDENTITY_NAME).is_err());
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o770
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let linked = root.path().join("linked");
        std::os::unix::fs::symlink(&path, &linked).unwrap();
        let linked_store = FileStore { root: linked };
        assert!(linked_store.get(IDENTITY_NAME).is_err());
        assert!(linked_store.put(IDENTITY_NAME, b"replacement").is_err());
        assert_eq!(store.get(IDENTITY_NAME).unwrap().unwrap(), b"fixture");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_release_launchers_share_identity_across_restarts_but_not_development() {
        let base = tempfile::tempdir().unwrap();
        let release = base
            .path()
            .join(runner_core::app_paths::app_identifier(false));
        let development = base
            .path()
            .join(runner_core::app_paths::app_identifier(true));
        let app = OsStore::new(&release, false);
        let identity = Identity::generate().unwrap();
        identity.save(&app).unwrap();
        let service = OsStore::new(&release, false);
        let cli = OsStore::new(&release, false);
        assert_eq!(
            Identity::load(&service).unwrap().unwrap().key.0,
            identity.key.0
        );
        assert_eq!(Identity::load(&cli).unwrap().unwrap().key.0, identity.key.0);
        let dev = OsStore::new(&development, true);
        assert!(Identity::load(&dev).unwrap().is_none());
        let dev_identity = Identity::generate().unwrap();
        dev_identity.save(&dev).unwrap();
        assert_ne!(dev_identity.key.0, identity.key.0);
        assert_eq!(Identity::load(&app).unwrap().unwrap().key.0, identity.key.0);
    }

    #[test]
    fn file_items_are_independent_private_and_reject_unsafe_files() {
        let root = tempfile::tempdir().unwrap();
        let store = FileStore {
            root: root.path().into(),
        };
        for (name, value) in [
            (IDENTITY_NAME, b"identity".as_slice()),
            (CLIENT_KEYS_NAME, b"client"),
            ("https://one.invalid", b"one"),
            ("https://two.invalid", b"two"),
        ] {
            store.put(name, value).unwrap();
            assert_eq!(store.get(name).unwrap().unwrap(), value);
            assert_eq!(
                std::fs::metadata(store.path(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        store.delete("https://one.invalid").unwrap();
        assert_eq!(store.get("https://two.invalid").unwrap().unwrap(), b"two");
        assert_eq!(store.get(IDENTITY_NAME).unwrap().unwrap(), b"identity");
        assert_eq!(store.get(CLIENT_KEYS_NAME).unwrap().unwrap(), b"client");
        let path = store.path(CLIENT_KEYS_NAME);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.get(CLIENT_KEYS_NAME).is_err());
        store.delete(CLIENT_KEYS_NAME).unwrap();
        std::os::unix::fs::symlink(store.path(IDENTITY_NAME), path).unwrap();
        assert!(store.get(CLIENT_KEYS_NAME).is_err());
    }
    #[test]
    fn mac_development_uses_files_and_production_uses_keychain() {
        let root = tempfile::tempdir().unwrap();
        let debug = OsStore::new(root.path(), true);
        let release = OsStore::new(root.path(), false);
        #[cfg(target_os = "macos")]
        {
            assert!(debug.file.is_some());
            assert!(release.file.is_none());
        }
        #[cfg(not(target_os = "macos"))]
        {
            assert!(debug.file.is_some());
            assert!(release.file.is_some());
        }
    }
}
