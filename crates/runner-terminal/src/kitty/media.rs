//! Local media loading (`t=f`, `t=t`, `t=s`) used by `runnerd`'s forwarders
//! before taking the session state lock.

use std::fs::File;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use super::command::{Command, Compression, Control, Medium, Transmission};
use super::image::{ImageError, MAX_DIMENSION, MAX_TRANSMITTED_BYTES};

/// Allowed temporary-directory roots for `t=t` (temporary file) transmissions.
#[derive(Debug, Clone, Default)]
pub struct TempDirs {
    extra: Vec<PathBuf>,
}

impl TempDirs {
    pub fn with_session_tmpdir(session_tmpdir: Option<&Path>) -> Self {
        let mut extra = Vec::new();
        extra.push(std::env::temp_dir());
        if let Some(dir) = session_tmpdir {
            if !dir.as_os_str().is_empty() {
                extra.push(dir.to_path_buf());
            }
        }
        Self { extra }
    }

    /// Construct with explicit roots (for tests that isolate their temp root).
    pub fn explicit(dirs: Vec<PathBuf>) -> Self {
        Self { extra: dirs }
    }

    fn allowed_roots(&self) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        #[cfg(unix)]
        {
            roots.push(PathBuf::from("/tmp"));
            roots.push(PathBuf::from("/dev/shm"));
            #[cfg(target_os = "macos")]
            roots.push(PathBuf::from("/private/tmp"));
        }
        for dir in &self.extra {
            roots.push(dir.clone());
            if let Ok(canon) = canonicalize_dir(dir) {
                roots.push(canon);
            }
        }
        roots
    }
}

/// A parsed Kitty command prepared by a forwarder thread for the authoritative
/// terminal model.
#[derive(Debug, Clone)]
pub struct PreparedCommand {
    pub command: Command,
    /// `Some` when the forwarder resolved a local medium (`t=f`, `t=t`, `t=s`).
    /// `None` for direct commands, non-transmission commands, or local-media
    /// commands that failed stateless header checks without touching any file.
    pub media: Option<Result<Vec<u8>, ImageError>>,
}

impl PreparedCommand {
    /// Parse a raw Kitty command without touching the local filesystem (used
    /// by mirrors, which only ever receive normalized direct commands, and by
    /// direct `TerminalModel::feed_output` calls outside the daemon forwarder).
    pub fn direct_only(raw: &[u8]) -> Option<Self> {
        let command = Command::parse(raw).ok()?;
        Some(Self {
            command,
            media: None,
        })
    }
}

/// Items handed from a session forwarder to `TerminalModel::feed_forwarded`.
#[derive(Debug, Clone)]
pub enum ForwardedItem {
    Text(Vec<u8>),
    Command(PreparedCommand),
    /// A command exceeded the scanner limit or failed syntax/base64 parsing.
    Discarded,
}

/// Parse a raw scanned Kitty command and, if it names a local medium whose
/// stateless header checks pass, read (and for `t=t`/`t=s`, delete/unlink) the
/// local resource outside the session state lock.
pub fn prepare_command(raw: &[u8], temp_dirs: &TempDirs) -> Option<PreparedCommand> {
    let command = Command::parse(raw).ok()?;
    let media = if should_read_media(&command) {
        Some(read_media(&command, temp_dirs))
    } else {
        None
    };
    Some(PreparedCommand { command, media })
}

/// Stateless header checks matching Ghostty before opening any local file or
/// shared memory object: `i` with `I`, `a=q` without `i`, unknown `f`, or a
/// medium unsupported on this platform. Returning `false` leaves any file
/// untouched and lets the model emit the exact Ghostty error reply.
pub fn should_read_media(cmd: &Command) -> bool {
    let ids = cmd.control.identifiers();
    if ids.image_id > 0 && ids.image_number > 0 {
        return false;
    }
    let t = match &cmd.control {
        Control::Query(t) => {
            if t.image_id == 0 {
                return false;
            }
            t
        }
        Control::Transmit(t)
        | Control::TransmitAndDisplay {
            transmission: t, ..
        } => t,
        _ => return false,
    };
    if t.medium == Medium::Direct {
        return false;
    }
    if t.format_unknown {
        return false;
    }
    if !medium_supported(t.medium) {
        return false;
    }
    true
}

pub fn medium_supported(medium: Medium) -> bool {
    match medium {
        Medium::Direct | Medium::File | Medium::TemporaryFile => true,
        Medium::SharedMemory => cfg!(all(unix, not(target_os = "android"))),
    }
}

pub fn read_media(cmd: &Command, temp_dirs: &TempDirs) -> Result<Vec<u8>, ImageError> {
    let t = cmd.control.transmission().ok_or(ImageError::InvalidData)?;
    match t.medium {
        Medium::Direct => Ok(cmd.data.clone()),
        Medium::File => read_file(false, t, &cmd.data, temp_dirs),
        Medium::TemporaryFile => read_file(true, t, &cmd.data, temp_dirs),
        Medium::SharedMemory => read_shared_memory(t, &cmd.data),
    }
}

fn read_file(
    temporary: bool,
    t: &Transmission,
    raw_path: &[u8],
    temp_dirs: &TempDirs,
) -> Result<Vec<u8>, ImageError> {
    if raw_path.is_empty() {
        return Err(ImageError::InvalidData);
    }

    #[cfg(not(windows))]
    if raw_path.contains(&0) {
        return Err(ImageError::InvalidData);
    }

    let path_str = std::str::from_utf8(raw_path).map_err(|_| ImageError::InvalidData)?;

    #[cfg(windows)]
    windows_path::check_path(path_str).map_err(|_| ImageError::InvalidData)?;

    let mut file = open_readonly_nonblocking(path_str).map_err(|_| ImageError::InvalidData)?;
    let canonical = canonical_path_from_file(&file).map_err(|_| ImageError::InvalidData)?;

    validate_canonical_path(&canonical)?;

    let mut delete_path: Option<PathBuf> = None;
    if temporary {
        if !is_path_in_any_dir(&temp_dirs.allowed_roots(), &canonical) {
            return Err(ImageError::TemporaryFileNotInTempDir);
        }
        let canon_str = canonical.to_string_lossy();
        if !canon_str.contains("tty-graphics-protocol") {
            return Err(ImageError::TemporaryFileNotNamedCorrectly);
        }
        delete_path = Some(canonical.clone());
    }

    // Read the file, ensuring the handle is closed before unlinking so Windows
    // can delete temporary files cleanly too.
    let read_result = (|| -> Result<Vec<u8>, ImageError> {
        let meta = file.metadata().map_err(|_| ImageError::InvalidData)?;
        if !meta.file_type().is_file() {
            return Err(ImageError::InvalidData);
        }
        if t.offset > 0 {
            file.seek(SeekFrom::Start(u64::from(t.offset)))
                .map_err(|_| ImageError::InvalidData)?;
        }
        if t.size > 0 {
            let size = usize::try_from(t.size).map_err(|_| ImageError::InvalidData)?;
            if size > MAX_TRANSMITTED_BYTES {
                return Err(ImageError::InvalidData);
            }
            let mut buf = vec![0u8; size];
            file.read_exact(&mut buf)
                .map_err(|_| ImageError::InvalidData)?;
            Ok(buf)
        } else {
            let mut buf = Vec::new();
            file.by_ref()
                .take(MAX_TRANSMITTED_BYTES as u64 + 1)
                .read_to_end(&mut buf)
                .map_err(|_| ImageError::InvalidData)?;
            if buf.len() > MAX_TRANSMITTED_BYTES {
                return Err(ImageError::InvalidData);
            }
            Ok(buf)
        }
    })();

    drop(file);
    if let Some(path) = delete_path {
        let _ = std::fs::remove_file(path);
    }

    read_result
}

#[cfg(unix)]
fn open_readonly_nonblocking(path: &str) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(path)
}

#[cfg(windows)]
fn open_readonly_nonblocking(path: &str) -> std::io::Result<File> {
    File::open(path)
}

#[cfg(target_os = "macos")]
fn canonical_path_from_file(file: &File) -> std::io::Result<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::io::AsRawFd as _;

    let mut buf = [0u8; libc::PATH_MAX as usize];
    let rc = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buf.as_mut_ptr()) };
    if rc == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let cstr = CStr::from_bytes_until_nul(&buf)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "missing NUL"))?;
    Ok(PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes())))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn canonical_path_from_file(file: &File) -> std::io::Result<PathBuf> {
    use std::os::unix::io::AsRawFd as _;
    let fd = file.as_raw_fd();
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
}

#[cfg(windows)]
fn canonical_path_from_file(file: &File) -> std::io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt as _;
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, VOLUME_NAME_DOS,
    };

    let handle = file.as_raw_handle() as HANDLE;
    let flags = FILE_NAME_NORMALIZED | VOLUME_NAME_DOS;
    let needed = unsafe { GetFinalPathNameByHandleW(handle, std::ptr::null_mut(), 0, flags) };
    if needed == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buf = vec![0u16; needed as usize];
    let written = unsafe { GetFinalPathNameByHandleW(handle, buf.as_mut_ptr(), needed, flags) };
    if written == 0 || written >= needed {
        return Err(std::io::Error::last_os_error());
    }
    let raw = std::ffi::OsString::from_wide(&buf[..written as usize]);
    let s = raw.to_string_lossy();
    let stripped = strip_windows_verbatim_drive(&s);
    Ok(PathBuf::from(stripped))
}

#[cfg(windows)]
fn strip_windows_verbatim_drive(path: &str) -> &str {
    // Strip `\\?\` only when followed by a drive-absolute path (`X:\...`),
    // keeping `\\?\UNC\...` and `\\?\Volume{...}` intact so check_canonical_path rejects them.
    if let Some(rest) = path.strip_prefix(r"\\?\") {
        if windows_path::is_drive_absolute(rest) {
            return rest;
        }
    }
    path
}

fn canonicalize_dir(dir: &Path) -> std::io::Result<PathBuf> {
    let canon = std::fs::canonicalize(dir)?;
    #[cfg(windows)]
    {
        let s = canon.to_string_lossy().into_owned();
        return Ok(PathBuf::from(strip_windows_verbatim_drive(&s)));
    }
    #[cfg(not(windows))]
    Ok(canon)
}

fn validate_canonical_path(path: &Path) -> Result<(), ImageError> {
    let text = path.to_str().ok_or(ImageError::InvalidData)?;
    #[cfg(windows)]
    {
        windows_path::check_canonical_path(text).map_err(|_| ImageError::InvalidData)
    }
    #[cfg(not(windows))]
    {
        if text == "/proc"
            || text.starts_with("/proc/")
            || text == "/sys"
            || text.starts_with("/sys/")
            || ((text == "/dev" || text.starts_with("/dev/")) && !text.starts_with("/dev/shm/"))
        {
            return Err(ImageError::InvalidData);
        }
        Ok(())
    }
}

fn is_path_in_any_dir(dirs: &[PathBuf], path: &Path) -> bool {
    let Some(path_str) = path.to_str() else {
        return false;
    };
    dirs.iter()
        .filter_map(|d| d.to_str())
        .any(|dir| is_path_in_dir(dir, path_str))
}

/// True if `path` equals `dir` or lies beneath it on a separator boundary.
pub fn is_path_in_dir(dir: &str, path: &str) -> bool {
    if dir.is_empty() {
        return false;
    }
    #[cfg(windows)]
    {
        let dir_norm = dir.replace('/', "\\");
        let path_norm = path.replace('/', "\\");
        let dir_lc = dir_norm.to_ascii_lowercase();
        let path_lc = path_norm.to_ascii_lowercase();
        if !path_lc.starts_with(&dir_lc) {
            return false;
        }
        if path_lc.len() == dir_lc.len() || dir_lc.ends_with('\\') {
            return true;
        }
        path_lc.as_bytes()[dir_lc.len()] == b'\\'
    }
    #[cfg(not(windows))]
    {
        if !path.starts_with(dir) {
            return false;
        }
        if path.len() == dir.len() || dir.ends_with('/') {
            return true;
        }
        path.as_bytes()[dir.len()] == b'/'
    }
}

fn valid_shm_name(name: &[u8], name_max: usize) -> bool {
    if name.is_empty() || name.len() > name_max || name == b"/" {
        return false;
    }
    !name.contains(&0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ShmRange {
    start: usize,
    end: usize,
}

fn shared_memory_range(t: &Transmission, stat_size: usize) -> Result<ShmRange, ImageError> {
    let expected_size = match t.format.raw_bpp() {
        None => None,
        Some(bpp) => {
            if t.width > MAX_DIMENSION || t.height > MAX_DIMENSION {
                return Err(ImageError::DimensionsTooLarge);
            }
            Some((t.width as usize) * (t.height as usize) * bpp)
        }
    };
    let start = usize::try_from(t.offset).map_err(|_| ImageError::InvalidData)?;
    if start > stat_size {
        return Err(ImageError::InvalidData);
    }
    let available = stat_size - start;
    let data_size = if t.size > 0 {
        usize::try_from(t.size).map_err(|_| ImageError::InvalidData)?
    } else if t.compression == Compression::None {
        expected_size.unwrap_or(available)
    } else {
        available
    };
    if data_size > MAX_TRANSMITTED_BYTES || data_size > available {
        return Err(ImageError::InvalidData);
    }
    Ok(ShmRange {
        start,
        end: start + data_size,
    })
}

#[cfg(all(unix, not(target_os = "android")))]
fn read_shared_memory(t: &Transmission, name: &[u8]) -> Result<Vec<u8>, ImageError> {
    use std::ffi::CString;

    // POSIX `NAME_MAX` (255 bytes); `libc::NAME_MAX` is not exported on macOS.
    const SHM_NAME_MAX: usize = 255;
    if !valid_shm_name(name, SHM_NAME_MAX) {
        return Err(ImageError::InvalidData);
    }
    let c_name = CString::new(name).map_err(|_| ImageError::InvalidData)?;
    let fd = unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0 as libc::c_uint) };
    if fd < 0 {
        return Err(ImageError::InvalidData);
    }

    struct ShmGuard {
        fd: libc::c_int,
        name: CString,
    }
    impl Drop for ShmGuard {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.fd);
                libc::shm_unlink(self.name.as_ptr());
            }
        }
    }
    let _guard = ShmGuard { fd, name: c_name };

    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(fd, &mut stat) } != 0 || stat.st_size <= 0 {
        return Err(ImageError::InvalidData);
    }
    let stat_size = usize::try_from(stat.st_size).map_err(|_| ImageError::InvalidData)?;
    let range = shared_memory_range(t, stat_size)?;
    if range.start == range.end {
        return Ok(Vec::new());
    }

    let map = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            stat_size,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if map == libc::MAP_FAILED {
        return Err(ImageError::InvalidData);
    }

    let bytes = unsafe {
        let slice = std::slice::from_raw_parts(map.cast::<u8>(), stat_size);
        slice[range.start..range.end].to_vec()
    };
    unsafe {
        libc::munmap(map, stat_size);
    }
    Ok(bytes)
}

#[cfg(not(all(unix, not(target_os = "android"))))]
fn read_shared_memory(_t: &Transmission, _name: &[u8]) -> Result<Vec<u8>, ImageError> {
    Err(ImageError::UnsupportedMedium)
}

/// Pure string Windows path checks following Ghostty `kitty/windows.zig`,
/// compiled and tested on every platform.
pub mod windows_path {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PathError {
        UncPath,
        DevicePath,
        ReservedDeviceName,
        NotDriveAbsolute,
    }

    fn is_sep(b: u8) -> bool {
        b == b'\\' || b == b'/'
    }

    pub fn is_drive_absolute(path: &str) -> bool {
        let b = path.as_bytes();
        b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
    }

    pub fn check_path(path: &str) -> Result<(), PathError> {
        let b = path.as_bytes();
        if b.len() >= 2 && is_sep(b[0]) && is_sep(b[1]) {
            if b.len() >= 4 && (b[2] == b'?' || b[2] == b'.') && is_sep(b[3]) {
                return Err(PathError::DevicePath);
            }
            return Err(PathError::UncPath);
        }
        if b.len() >= 4 && is_sep(b[0]) && b[1] == b'?' && b[2] == b'?' && is_sep(b[3]) {
            return Err(PathError::DevicePath);
        }
        for component in path.split(['\\', '/']) {
            if is_reserved_device_name(component) {
                return Err(PathError::ReservedDeviceName);
            }
        }
        Ok(())
    }

    pub fn check_canonical_path(path: &str) -> Result<(), PathError> {
        if !is_drive_absolute(path) {
            return Err(PathError::NotDriveAbsolute);
        }
        check_path(path)
    }

    fn is_reserved_device_name(component: &str) -> bool {
        let end = component.find(['.', ':']).unwrap_or(component.len());
        let stem = component[..end].trim_end_matches(' ');
        if stem.len() < 3 {
            return false;
        }
        let bytes = stem.as_bytes();
        let base = &bytes[..3];
        let rest = &bytes[3..];
        if rest.is_empty() {
            return base.eq_ignore_ascii_case(b"CON")
                || base.eq_ignore_ascii_case(b"PRN")
                || base.eq_ignore_ascii_case(b"AUX")
                || base.eq_ignore_ascii_case(b"NUL");
        }
        if !base.eq_ignore_ascii_case(b"COM") && !base.eq_ignore_ascii_case(b"LPT") {
            return false;
        }
        match rest {
            [d] => d.is_ascii_digit(),
            // UTF-8 superscript digits ¹ ² ³ (U+00B9, U+00B2, U+00B3)
            [0xc2, 0xb9 | 0xb2 | 0xb3] => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;

    fn b64(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn windows_path_rules_match_ghostty() {
        use windows_path::{check_canonical_path, check_path, is_drive_absolute, PathError};

        for unc in [
            r"\\server\share\tty-graphics-protocol-image.data",
            "//server/share/tty-graphics-protocol-image.data",
            r"\/server/share/image.data",
            r"\\",
        ] {
            assert_eq!(check_path(unc), Err(PathError::UncPath), "{unc}");
        }
        for dev in [
            r"\\?\UNC\server\share\image.data",
            r"\\?\C:\Windows\Temp\image.data",
            r"\\.\pipe\image",
            r"\??\C:\Windows\Temp\image.data",
            "/??/C:/image.data",
        ] {
            assert_eq!(check_path(dev), Err(PathError::DevicePath), "{dev}");
        }
        for reserved in [
            "CON",
            "CON.png",
            "NUL:stream",
            "COM1",
            "LPT9",
            "COM\u{00b9}",
            "LPT\u{00b3}.png",
            r"C:\CON\image.data",
            r"..\PRN",
        ] {
            assert_eq!(
                check_path(reserved),
                Err(PathError::ReservedDeviceName),
                "{reserved}"
            );
        }
        for ok in [
            r"C:\Windows\Temp\tty-graphics-protocol-image.data",
            "CONSOLE",
            "COM10",
            "image.data",
        ] {
            assert_eq!(check_path(ok), Ok(()), "{ok}");
        }
        assert_eq!(check_canonical_path(r"C:\Temp\a.data"), Ok(()));
        assert_eq!(
            check_canonical_path(r"\\?\Volume{1}\a.data"),
            Err(PathError::NotDriveAbsolute)
        );
        assert!(is_drive_absolute(r"C:\Temp\a.data"));
        assert!(!is_drive_absolute("C:/Temp/a.data"));
    }

    #[test]
    fn path_prefix_requires_separator_boundary() {
        assert!(is_path_in_dir("/tmp", "/tmp/tty-graphics-protocol-a"));
        assert!(is_path_in_dir("/tmp/", "/tmp/tty-graphics-protocol-a"));
        assert!(!is_path_in_dir("/tmp", "/tmpX/tty-graphics-protocol-a"));
        assert!(!is_path_in_dir(
            "/dev/shm",
            "/dev/shm-evil/tty-graphics-protocol"
        ));
        assert!(!is_path_in_dir("", "/tmp/a"));
    }

    #[test]
    fn shared_memory_name_and_range_validation() {
        assert!(valid_shm_name(b"/kitty", 8));
        assert!(valid_shm_name(b"kitty", 8));
        assert!(!valid_shm_name(b"", 8));
        assert!(!valid_shm_name(b"/", 8));
        assert!(!valid_shm_name(b"/kitty\0x", 16));
        assert!(!valid_shm_name(b"/12345678", 8));

        let t = Transmission {
            format: super::super::command::Format::Rgb,
            width: 1,
            height: 1,
            offset: 2,
            size: 3,
            ..Default::default()
        };
        assert_eq!(
            shared_memory_range(&t, 5),
            Ok(ShmRange { start: 2, end: 5 })
        );
        let t_implicit = Transmission {
            format: super::super::command::Format::Rgb,
            width: 1,
            height: 1,
            offset: 2,
            size: 0,
            ..Default::default()
        };
        assert_eq!(
            shared_memory_range(&t_implicit, 4096),
            Ok(ShmRange { start: 2, end: 5 })
        );
        let t_oob = Transmission {
            offset: 4,
            ..t_implicit
        };
        assert_eq!(shared_memory_range(&t_oob, 3), Err(ImageError::InvalidData));
        let t_huge = Transmission {
            width: u32::MAX,
            height: u32::MAX,
            ..t_implicit
        };
        assert_eq!(
            shared_memory_range(&t_huge, 1),
            Err(ImageError::DimensionsTooLarge)
        );
    }

    #[test]
    fn header_checks_leave_temp_file_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tty-graphics-protocol-header-check.data");
        std::fs::write(&file, [1u8, 2, 3]).unwrap();
        let payload = b64(file.to_str().unwrap().as_bytes());
        let temp_dirs = TempDirs::explicit(vec![dir.path().to_path_buf()]);

        for raw in [
            format!("a=t,t=t,f=24,s=1,v=1,i=1,I=2;{payload}"),
            format!("a=q,t=t,f=24,s=1,v=1;{payload}"),
            format!("a=t,t=t,f=42,s=1,v=1,i=1;{payload}"),
        ] {
            let prep = prepare_command(raw.as_bytes(), &temp_dirs).unwrap();
            assert!(prep.media.is_none(), "{raw}");
            assert!(file.exists(), "file must remain untouched for {raw}");
        }
    }

    #[test]
    fn regular_and_temporary_file_reads_offsets_sizes_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("image.data");
        std::fs::write(&file, [10u8, 20, 30, 40, 50, 60]).unwrap();
        let payload = b64(file.to_str().unwrap().as_bytes());
        let temp_dirs = TempDirs::explicit(vec![dir.path().to_path_buf()]);

        // Regular file with S and O keeps the file.
        let prep = prepare_command(
            format!("a=t,t=f,f=24,s=1,v=1,i=1,O=2,S=3;{payload}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(prep.media, Some(Ok(vec![30, 40, 50])));
        assert!(file.exists());

        // Exact S larger than remaining bytes fails.
        let prep = prepare_command(
            format!("a=t,t=f,f=24,s=1,v=1,i=1,O=4,S=5;{payload}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(prep.media, Some(Err(ImageError::InvalidData)));

        // Temporary file with wrong name is refused and NOT deleted.
        let prep = prepare_command(
            format!("a=t,t=t,f=24,s=1,v=1,i=1;{payload}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(
            prep.media,
            Some(Err(ImageError::TemporaryFileNotNamedCorrectly))
        );
        assert!(file.exists());

        // Temporary file outside allowed temp dir is refused and NOT deleted.
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("tty-graphics-protocol-outside.data");
        std::fs::write(&outside_file, [1u8, 2, 3]).unwrap();
        let outside_payload = b64(outside_file.to_str().unwrap().as_bytes());
        let restricted = TempDirs::explicit(vec![dir.path().to_path_buf()]);
        // Only assert NotInTempDir if `outside` isn't itself under /tmp (macOS tempdirs are under /var/folders).
        if !is_path_in_any_dir(
            &restricted.allowed_roots(),
            &std::fs::canonicalize(&outside_file).unwrap(),
        ) {
            let prep = prepare_command(
                format!("a=t,t=t,f=24,s=1,v=1,i=1;{outside_payload}").as_bytes(),
                &restricted,
            )
            .unwrap();
            assert_eq!(prep.media, Some(Err(ImageError::TemporaryFileNotInTempDir)));
            assert!(outside_file.exists());
        }

        // Valid temporary file is deleted after reading, even when S fails.
        let temp_file = dir.path().join("tty-graphics-protocol-ok.data");
        std::fs::write(&temp_file, [7u8, 8, 9]).unwrap();
        let temp_payload = b64(temp_file.to_str().unwrap().as_bytes());
        let prep = prepare_command(
            format!("a=t,t=t,f=24,s=1,v=1,i=1,S=99;{temp_payload}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(prep.media, Some(Err(ImageError::InvalidData)));
        assert!(
            !temp_file.exists(),
            "temp file must be deleted even on read failure"
        );

        // Directory is refused.
        let dir_payload = b64(dir.path().to_str().unwrap().as_bytes());
        let prep = prepare_command(
            format!("a=t,t=f,f=24,s=1,v=1,i=1;{dir_payload}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(prep.media, Some(Err(ImageError::InvalidData)));
    }

    #[cfg(unix)]
    #[test]
    fn unix_file_blocklist_fifo_symlink_and_shm_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let temp_dirs = TempDirs::explicit(vec![dir.path().to_path_buf()]);

        // /dev/null is refused (under /dev outside /dev/shm).
        let dev_null = b64(b"/dev/null");
        let prep = prepare_command(
            format!("a=t,t=f,f=24,s=1,v=1,i=1;{dev_null}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(prep.media, Some(Err(ImageError::InvalidData)));

        // Symlink to /dev/null is refused (canonical path checked from open fd).
        let link = dir.path().join("sneaky-link");
        std::os::unix::fs::symlink("/dev/null", &link).unwrap();
        let link_b64 = b64(link.to_str().unwrap().as_bytes());
        let prep = prepare_command(
            format!("a=t,t=f,f=24,s=1,v=1,i=1;{link_b64}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(prep.media, Some(Err(ImageError::InvalidData)));

        // /proc (when present, e.g. Linux) and symlink into /proc are refused.
        if Path::new("/proc/self/cmdline").exists() {
            let proc_link = dir.path().join("proc-link");
            std::os::unix::fs::symlink("/proc/self/cmdline", &proc_link).unwrap();
            let proc_b64 = b64(proc_link.to_str().unwrap().as_bytes());
            let prep = prepare_command(
                format!("a=t,t=f,f=24,s=1,v=1,i=1;{proc_b64}").as_bytes(),
                &temp_dirs,
            )
            .unwrap();
            assert_eq!(prep.media, Some(Err(ImageError::InvalidData)));
        }

        // FIFO is refused immediately without blocking.
        let fifo = dir.path().join("pipe");
        let c_fifo = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(c_fifo.as_ptr(), 0o600) }, 0);
        let fifo_b64 = b64(fifo.to_str().unwrap().as_bytes());
        let prep = prepare_command(
            format!("a=t,t=f,f=24,s=1,v=1,i=1;{fifo_b64}").as_bytes(),
            &temp_dirs,
        )
        .unwrap();
        assert_eq!(prep.media, Some(Err(ImageError::InvalidData)));

        // Shared memory round-trip and unlink.
        let shm_name = format!("/rnr-k-{}-{}", std::process::id(), 1);
        let c_name = std::ffi::CString::new(shm_name.as_str()).unwrap();
        let fd = unsafe {
            libc::shm_open(
                c_name.as_ptr(),
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
                0o600 as libc::c_uint,
            )
        };
        if fd >= 0 {
            let payload = [11u8, 22, 33];
            assert_eq!(
                unsafe { libc::ftruncate(fd, payload.len() as libc::off_t) },
                0
            );
            let map = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    payload.len(),
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd,
                    0,
                )
            };
            assert_ne!(map, libc::MAP_FAILED);
            unsafe {
                std::ptr::copy_nonoverlapping(payload.as_ptr(), map.cast::<u8>(), payload.len());
                libc::munmap(map, payload.len());
                libc::close(fd);
            }

            let shm_b64 = b64(shm_name.as_bytes());
            let prep = prepare_command(
                format!("a=t,t=s,f=24,s=1,v=1,i=1;{shm_b64}").as_bytes(),
                &temp_dirs,
            )
            .unwrap();
            assert_eq!(prep.media, Some(Ok(payload.to_vec())));

            // Must have been unlinked.
            let reopened =
                unsafe { libc::shm_open(c_name.as_ptr(), libc::O_RDONLY, 0 as libc::c_uint) };
            assert!(reopened < 0, "shm object should have been unlinked");
        }
    }
}
