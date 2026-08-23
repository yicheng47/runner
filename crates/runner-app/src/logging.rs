use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};
use tracing_subscriber::prelude::*;

const LOG_FILE_NAME: &str = "runner.log";
const MAX_LOG_BYTES: u64 = 10 * 1024 * 1024;
const LOG_BACKUPS: usize = 3;
static TRACING_INSTALLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone)]
struct SharedLogWriter(Arc<Mutex<RotatingFile>>);

impl Write for SharedLogWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log writer poisoned").write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().expect("log writer poisoned").flush()
    }
}

struct RotatingFile {
    path: PathBuf,
    file: Option<File>,
    bytes: u64,
    max_bytes: u64,
    backups: usize,
}

impl RotatingFile {
    fn open(path: PathBuf, max_bytes: u64, backups: usize) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            path,
            file: Some(file),
            bytes,
            max_bytes,
            backups,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
        }
        if self.backups > 0 {
            let oldest = backup_path(&self.path, self.backups);
            if oldest.exists() {
                std::fs::remove_file(oldest)?;
            }
            for index in (1..self.backups).rev() {
                let source = backup_path(&self.path, index);
                if source.exists() {
                    std::fs::rename(source, backup_path(&self.path, index + 1))?;
                }
            }
            if self.path.exists() {
                std::fs::rename(&self.path, backup_path(&self.path, 1))?;
            }
        }
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)?,
        );
        self.bytes = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes > 0 && self.bytes.saturating_add(bytes.len() as u64) > self.max_bytes {
            self.rotate()?;
        }
        let written = self.file.as_mut().expect("log file missing").write(bytes)?;
        self.bytes = self.bytes.saturating_add(written as u64);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().expect("log file missing").flush()
    }
}

fn backup_path(path: &Path, index: usize) -> PathBuf {
    PathBuf::from(format!("{}.{}", path.display(), index))
}

pub fn install(log_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("create log directory {}", log_dir.display()))?;
    let log_path = log_dir.join(LOG_FILE_NAME);
    install_panic_hook(log_path.clone());

    let writer = SharedLogWriter(Arc::new(Mutex::new(
        RotatingFile::open(log_path, MAX_LOG_BYTES, LOG_BACKUPS)
            .context("open rotating Runner log")?,
    )));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new(if cfg!(debug_assertions) {
            "debug"
        } else {
            "info"
        })
    });
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(move || writer.clone());
    let stderr_layer = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer()
            .with_ansi(true)
            .with_writer(io::stderr)
    });
    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .context("install tracing subscriber")?;
    TRACING_INSTALLED.store(true, Ordering::Release);
    Ok(())
}

pub fn startup_banner(version: &str, app_data_dir: &Path) {
    tracing::info!(
        "starting Runner v{} on {}-{}; app_data_dir={}",
        version,
        std::env::consts::OS,
        std::env::consts::ARCH,
        app_data_dir.display()
    );
}

fn install_panic_hook(fallback_path: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        if TRACING_INSTALLED.load(Ordering::Acquire) {
            tracing::error!("panic: {info}\n{backtrace}");
        } else {
            let _ = write_panic_fallback(&fallback_path, &info.to_string(), &backtrace);
        }
        previous(info);
    }));
}

fn write_panic_fallback(
    path: &Path,
    message: &str,
    backtrace: &std::backtrace::Backtrace,
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(
        file,
        "[{}] panic: {message}\n{backtrace}",
        chrono::Utc::now().to_rfc3339()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_rotates_at_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        let mut writer = RotatingFile::open(path.clone(), 8, 2).unwrap();
        writer.write_all(b"12345678").unwrap();
        writer.write_all(b"next").unwrap();
        writer.flush().unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "next");
        assert_eq!(
            std::fs::read_to_string(backup_path(&path, 1)).unwrap(),
            "12345678"
        );
    }

    #[test]
    fn panic_fallback_writes_the_panic_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOG_FILE_NAME);
        write_panic_fallback(
            &path,
            "forced-panic-marker",
            &std::backtrace::Backtrace::disabled(),
        )
        .unwrap();
        assert!(std::fs::read_to_string(path)
            .unwrap()
            .contains("forced-panic-marker"));
    }
}
