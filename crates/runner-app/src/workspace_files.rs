use std::{
    fs,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};

const MAX_DIRECTORY_ENTRIES: usize = 10_000;
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub ignored: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewContent {
    Markdown(String),
    Text(String),
    Image,
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FilePreview {
    pub path: PathBuf,
    pub bytes: u64,
    pub content: PreviewContent,
}

/// Resolve a workspace path to its canonical filesystem identity.
/// Relative paths are interpreted from the workspace root.
pub fn resolve_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("Cannot access workspace root: {error}"))?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let resolved = candidate
        .canonicalize()
        .map_err(|error| format!("Cannot access path: {error}"))?;
    if !resolved.starts_with(&root) {
        return Err("Path resolves outside the workspace".into());
    }
    if resolved
        .strip_prefix(&root)
        .ok()
        .is_some_and(|relative| relative.components().any(|part| part.as_os_str() == ".git"))
    {
        return Err("The .git directory is not accessible from the workspace panel".into());
    }
    Ok(resolved)
}

/// Read one directory only. Symlink paths are retained for tree display while
/// their targets are classified without recursively enumerating them.
pub fn read_directory(root: &Path, path: &Path) -> Result<Vec<DirectoryEntry>, String> {
    read_directory_after_resolve(root, path, || {})
}

#[cfg(test)]
fn read_directory_with_limit(
    root: &Path,
    path: &Path,
    max_entries: usize,
) -> Result<Vec<DirectoryEntry>, String> {
    read_directory_after_resolve_with_limit(root, path, max_entries, || {})
}

fn read_directory_after_resolve<F>(
    root: &Path,
    path: &Path,
    after_resolve: F,
) -> Result<Vec<DirectoryEntry>, String>
where
    F: FnOnce(),
{
    read_directory_after_resolve_with_limit(root, path, MAX_DIRECTORY_ENTRIES, after_resolve)
}

fn read_directory_after_resolve_with_limit<F>(
    root: &Path,
    path: &Path,
    max_entries: usize,
    after_resolve: F,
) -> Result<Vec<DirectoryEntry>, String>
where
    F: FnOnce(),
{
    let root_canonical = root
        .canonicalize()
        .map_err(|error| format!("Cannot access workspace root: {error}"))?;
    let resolved_dir = resolve_path(&root_canonical, path)?;
    after_resolve();
    let (directory_handle, actual_dir) = open_verified_directory(&root_canonical, &resolved_dir)?;
    let mut entries = read_verified_directory_entries(
        &root_canonical,
        &directory_handle,
        &actual_dir,
        max_entries,
    )?;
    mark_ignored_entries(&root_canonical, &mut entries);
    entries.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(entries)
}

/// Return a bounded preview for a regular file under the workspace root.
pub fn read_preview(root: &Path, path: &Path) -> Result<FilePreview, String> {
    read_preview_after_resolve(root, path, || {})
}

/// Read verified image bytes for a preview renderer without reopening a path
/// that may have changed since its metadata was checked.
pub fn read_image_bytes(root: &Path, path: &Path) -> Result<Vec<u8>, String> {
    read_image_bytes_after_resolve(root, path, || {})
}

fn read_image_bytes_after_resolve<F>(
    root: &Path,
    path: &Path,
    after_resolve: F,
) -> Result<Vec<u8>, String>
where
    F: FnOnce(),
{
    let root_canonical = root
        .canonicalize()
        .map_err(|error| format!("Cannot access workspace root: {error}"))?;
    let resolved = resolve_path(&root_canonical, path)?;
    after_resolve();
    read_verified_image_bytes(&root_canonical, &resolved)
}

fn read_preview_after_resolve<F>(
    root: &Path,
    path: &Path,
    after_resolve: F,
) -> Result<FilePreview, String>
where
    F: FnOnce(),
{
    let root_canonical = root
        .canonicalize()
        .map_err(|error| format!("Cannot access workspace root: {error}"))?;
    let resolved = resolve_path(&root_canonical, path)?;
    after_resolve();
    let (file, actual_path) = open_verified_file(&root_canonical, &resolved)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect opened file: {error}"))?;
    if !metadata.is_file() {
        return Err("Only regular files can be previewed".into());
    }
    let bytes = metadata.len();
    let extension = actual_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let content = if is_explicitly_unsupported(&extension) {
        PreviewContent::Unsupported(format!(".{} files are not previewed", extension))
    } else if is_raster_extension(&extension) {
        if bytes > MAX_IMAGE_BYTES {
            PreviewContent::Unsupported(format!(
                "Image exceeds the {MAX_IMAGE_BYTES}-byte preview limit"
            ))
        } else {
            match read_bounded(file, MAX_IMAGE_BYTES as usize) {
                Ok(contents) if has_matching_image_signature(&extension, &contents) => {
                    PreviewContent::Image
                }
                Ok(_) => PreviewContent::Unsupported(
                    "Image data is invalid or does not match its extension".into(),
                ),
                Err(error) => return Err(format!("Cannot read file: {error}")),
            }
        }
    } else if bytes > MAX_TEXT_BYTES as u64 {
        PreviewContent::Unsupported(format!(
            "File exceeds the {MAX_TEXT_BYTES}-byte text preview limit"
        ))
    } else {
        match read_bounded(file, MAX_TEXT_BYTES) {
            Ok(contents) => match String::from_utf8(contents) {
                Ok(text) if text.contains('\0') => {
                    PreviewContent::Unsupported("File contains binary NUL bytes".into())
                }
                Ok(text) if extension == "md" || extension == "markdown" => {
                    PreviewContent::Markdown(text)
                }
                Ok(text) => PreviewContent::Text(text),
                Err(_) => PreviewContent::Unsupported("File is not valid UTF-8 text".into()),
            },
            Err(error) => return Err(format!("Cannot read file: {error}")),
        }
    };
    Ok(FilePreview {
        path: actual_path,
        bytes,
        content,
    })
}

fn read_verified_image_bytes(root: &Path, resolved: &Path) -> Result<Vec<u8>, String> {
    let (file, actual_path) = open_verified_file(root, resolved)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect opened image: {error}"))?;
    if !metadata.is_file() {
        return Err("Only regular files can be previewed as images".into());
    }
    if metadata.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "Image exceeds the {MAX_IMAGE_BYTES}-byte preview limit"
        ));
    }
    let extension = actual_path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !is_raster_extension(&extension) {
        return Err("File extension is not a supported raster image".into());
    }
    let bytes = read_bounded(file, MAX_IMAGE_BYTES as usize)
        .map_err(|error| format!("Cannot read image: {error}"))?;
    if !has_matching_image_signature(&extension, &bytes) {
        return Err("Image data is invalid or does not match its extension".into());
    }
    Ok(bytes)
}

fn open_verified_file(root: &Path, resolved: &Path) -> Result<(File, PathBuf), String> {
    let file = open_path_no_follow(resolved, false)
        .map_err(|error| format!("Cannot open verified file: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("Cannot inspect opened file: {error}"))?;
    if !metadata.is_file() {
        return Err("Only regular files can be previewed".into());
    }
    let actual = opened_path(&file)?;
    validate_opened_path(root, &actual)?;
    Ok((file, actual))
}

fn open_verified_directory(root: &Path, resolved: &Path) -> Result<(File, PathBuf), String> {
    let directory = open_path_no_follow(resolved, true)
        .map_err(|error| format!("Cannot open verified directory: {error}"))?;
    if !directory
        .metadata()
        .map_err(|error| format!("Cannot inspect opened directory: {error}"))?
        .is_dir()
    {
        return Err("Path is not a directory".into());
    }
    let actual = opened_path(&directory)?;
    validate_opened_path(root, &actual)?;
    Ok((directory, actual))
}

fn validate_opened_path(root: &Path, actual: &Path) -> Result<(), String> {
    if !opened_path_is_within(root, actual) {
        return Err("Opened path resolves outside the workspace".into());
    }
    if opened_path_contains_git(root, actual) {
        return Err("The .git directory is not accessible from the workspace panel".into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn opened_path_is_within(root: &Path, actual: &Path) -> bool {
    actual.starts_with(root)
}

#[cfg(windows)]
fn opened_path_is_within(root: &Path, actual: &Path) -> bool {
    let root = windows_path_key(root);
    let actual = windows_path_key(actual);
    actual == root
        || actual
            .strip_prefix(&root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

#[cfg(not(windows))]
fn opened_path_contains_git(root: &Path, actual: &Path) -> bool {
    actual
        .strip_prefix(root)
        .ok()
        .is_some_and(|relative| relative.components().any(|part| part.as_os_str() == ".git"))
}

#[cfg(windows)]
fn opened_path_contains_git(root: &Path, actual: &Path) -> bool {
    let root = windows_path_key(root);
    let actual = windows_path_key(actual);
    actual.strip_prefix(&root).is_some_and(|suffix| {
        suffix
            .split('\\')
            .any(|component| component.eq_ignore_ascii_case(".git"))
    })
}

#[cfg(windows)]
fn windows_path_key(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn open_path_no_follow(path: &Path, directory: bool) -> io::Result<File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        let mut flags = libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
        if directory {
            flags |= libc::O_DIRECTORY;
        }
        options.custom_flags(flags);
    }
    #[cfg(windows)]
    {
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        let mut flags = FILE_FLAG_OPEN_REPARSE_POINT;
        if directory {
            flags |= FILE_FLAG_BACKUP_SEMANTICS;
        }
        options
            .custom_flags(flags)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = directory;
    options.open(path)
}

#[cfg(target_os = "linux")]
fn opened_path(file: &File) -> Result<PathBuf, String> {
    fs::canonicalize(format!("/proc/self/fd/{}", file.as_raw_fd()))
        .map_err(|error| format!("Cannot verify opened file path: {error}"))
}

#[cfg(target_os = "macos")]
fn opened_path(file: &File) -> Result<PathBuf, String> {
    let mut buffer = vec![0i8; libc::PATH_MAX as usize];
    // SAFETY: F_GETPATH writes at most PATH_MAX bytes into the provided buffer.
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
    if result == -1 {
        return Err(format!(
            "Cannot verify opened file path: {}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: F_GETPATH returns a NUL-terminated path on success.
    let path = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
    use std::os::unix::ffi::OsStringExt;
    Ok(PathBuf::from(std::ffi::OsString::from_vec(
        path.to_bytes().to_vec(),
    )))
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn opened_path(_file: &File) -> Result<PathBuf, String> {
    Err("Opened path verification is unsupported on this Unix platform".into())
}

#[cfg(unix)]
fn read_verified_directory_entries(
    _root: &Path,
    directory: &File,
    actual_dir: &Path,
    max_entries: usize,
) -> Result<Vec<DirectoryEntry>, String> {
    use std::{
        ffi::{CStr, OsStr},
        os::unix::ffi::{OsStrExt, OsStringExt},
    };

    // fdopendir owns this duplicate while preserving the already-verified directory inode.
    // SAFETY: dup receives a live borrowed fd and returns an independently owned fd.
    let duplicate = unsafe { libc::dup(directory.as_raw_fd()) };
    if duplicate == -1 {
        return Err(format!(
            "Cannot duplicate verified directory handle: {}",
            io::Error::last_os_error()
        ));
    }
    // SAFETY: duplicate is valid and ownership transfers to fdopendir on success.
    let stream = unsafe { libc::fdopendir(duplicate) };
    if stream.is_null() {
        // SAFETY: fdopendir failed, so duplicate remains ours to close.
        unsafe { libc::close(duplicate) };
        return Err(format!(
            "Cannot enumerate verified directory: {}",
            io::Error::last_os_error()
        ));
    }

    let mut entries = Vec::new();
    loop {
        // SAFETY: stream is a live DIR* owned by this function until closed below.
        let item = unsafe { libc::readdir(stream) };
        if item.is_null() {
            break;
        }
        // SAFETY: readdir returns an entry whose d_name is NUL-terminated and valid until next call.
        let name_bytes = unsafe { CStr::from_ptr((*item).d_name.as_ptr()) }.to_bytes();
        if name_bytes == b"." || name_bytes == b".." || name_bytes == b".git" {
            continue;
        }
        if entries.len() == max_entries {
            // SAFETY: stream is live and is closed exactly once on this return path.
            unsafe { libc::closedir(stream) };
            return Err(format!("Directory exceeds the {max_entries}-entry limit"));
        }
        let name_os = std::ffi::OsString::from_vec(name_bytes.to_vec());
        let name = name_os.to_string_lossy().into_owned();
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let name_c = std::ffi::CString::new(OsStr::new(&name_os).as_bytes())
            .map_err(|_| "Directory entry name contains NUL".to_owned())?;
        // SAFETY: name_c is NUL-terminated; stat is valid writable storage.
        let result = unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                name_c.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == -1 {
            // SAFETY: stream is live and is closed exactly once on this return path.
            unsafe { libc::closedir(stream) };
            return Err(format!(
                "Cannot inspect {name}: {}",
                io::Error::last_os_error()
            ));
        }
        // SAFETY: successful fstatat initialized stat.
        let stat = unsafe { stat.assume_init() };
        let is_symlink = stat.st_mode & libc::S_IFMT == libc::S_IFLNK;
        let is_dir = if is_symlink {
            let mut target_stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: name_c is NUL-terminated; target_stat is valid writable storage.
            let result = unsafe {
                libc::fstatat(
                    directory.as_raw_fd(),
                    name_c.as_ptr(),
                    target_stat.as_mut_ptr(),
                    0,
                )
            };
            result == 0 && {
                // SAFETY: successful fstatat initialized target_stat.
                unsafe { target_stat.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFDIR
            }
        } else {
            stat.st_mode & libc::S_IFMT == libc::S_IFDIR
        };
        entries.push(DirectoryEntry {
            path: actual_dir.join(&name_os),
            name,
            is_dir,
            is_symlink,
            ignored: false,
        });
    }
    // SAFETY: stream is live and all entry names have been copied out.
    unsafe { libc::closedir(stream) };
    Ok(entries)
}

#[cfg(windows)]
fn read_verified_directory_entries(
    _root: &Path,
    directory: &File,
    actual_dir: &Path,
    max_entries: usize,
) -> Result<Vec<DirectoryEntry>, String> {
    use std::{
        ffi::{c_void, OsString},
        os::windows::ffi::OsStringExt,
    };

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandleEx(
            file: *mut c_void,
            information_class: u32,
            information: *mut c_void,
            buffer_size: u32,
        ) -> i32;
        fn GetLastError() -> u32;
    }

    #[repr(C)]
    struct FileIdBothDirectoryInfo {
        next_entry_offset: u32,
        _file_index: u32,
        _creation_time: i64,
        _last_access_time: i64,
        _last_write_time: i64,
        _change_time: i64,
        _end_of_file: i64,
        _allocation_size: i64,
        file_attributes: u32,
        file_name_length: u32,
        _ea_size: u32,
        _short_name_length: i8,
        _short_name: [u16; 12],
        _file_id: i64,
        file_name: [u16; 1],
    }

    const FILE_ID_BOTH_DIR_INFO: u32 = 10;
    const ERROR_NO_MORE_FILES: u32 = 18;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    const BUFFER_SIZE: usize = 64 * 1024;

    let mut buffer = vec![0u64; BUFFER_SIZE / std::mem::size_of::<u64>()];
    let buffer_ptr = buffer.as_mut_ptr().cast::<u8>();
    let handle = directory.as_raw_handle() as *mut c_void;
    let mut entries = Vec::new();
    loop {
        // SAFETY: handle is an open directory handle and buffer is writable for BUFFER_SIZE bytes.
        let success = unsafe {
            GetFileInformationByHandleEx(
                handle,
                FILE_ID_BOTH_DIR_INFO,
                buffer_ptr.cast(),
                BUFFER_SIZE as u32,
            )
        };
        if success == 0 {
            // SAFETY: GetLastError has no preconditions.
            let error = unsafe { GetLastError() };
            if error == ERROR_NO_MORE_FILES {
                break;
            }
            return Err(format!(
                "Cannot enumerate verified directory: Windows error {error}"
            ));
        }

        let mut offset = 0usize;
        loop {
            if offset + std::mem::size_of::<FileIdBothDirectoryInfo>() - 2 > BUFFER_SIZE {
                return Err("Invalid directory enumeration buffer from Windows".into());
            }
            // SAFETY: buffer bounds and record offset were validated before dereferencing.
            let info = unsafe { &*buffer_ptr.add(offset).cast::<FileIdBothDirectoryInfo>() };
            let name_units = (info.file_name_length / 2) as usize;
            let name_offset = std::mem::offset_of!(FileIdBothDirectoryInfo, file_name);
            if info.file_name_length % 2 != 0 || offset + name_offset + name_units * 2 > BUFFER_SIZE
            {
                return Err("Invalid directory entry name from Windows".into());
            }
            // SAFETY: validated name range lies inside initialized enumeration buffer.
            let name = unsafe {
                std::slice::from_raw_parts(
                    buffer_ptr.add(offset + name_offset).cast::<u16>(),
                    name_units,
                )
            };
            let name = OsString::from_wide(name);
            if name != "." && name != ".." && name != ".git" {
                if entries.len() == max_entries {
                    return Err(format!("Directory exceeds the {max_entries}-entry limit"));
                }
                let path = actual_dir.join(&name);
                let attributes = info.file_attributes;
                let is_symlink = attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0;
                let is_dir = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
                entries.push(DirectoryEntry {
                    name: name.to_string_lossy().into_owned(),
                    path,
                    is_dir,
                    is_symlink,
                    ignored: false,
                });
            }
            if info.next_entry_offset == 0 {
                break;
            }
            let next = info.next_entry_offset as usize;
            if next == 0 || offset + next >= BUFFER_SIZE {
                return Err("Invalid directory entry chain from Windows".into());
            }
            offset += next;
        }
    }
    Ok(entries)
}

#[cfg(windows)]
fn opened_path(file: &File) -> Result<PathBuf, String> {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFinalPathNameByHandleW(
            file: *mut c_void,
            path: *mut u16,
            path_len: u32,
            flags: u32,
        ) -> u32;
    }

    let mut buffer = vec![0u16; 32768];
    // FILE_NAME_NORMALIZED | VOLUME_NAME_DOS are both zero-valued flags.
    // SAFETY: file is a valid handle and buffer is writable for its declared length.
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as *mut c_void,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    } as usize;
    if length == 0 || length >= buffer.len() {
        return Err("Cannot verify opened file path: path buffer was insufficient".into());
    }
    let path = String::from_utf16(&buffer[..length])
        .map_err(|error| format!("Cannot decode opened file path: {error}"))?;
    let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
    Ok(PathBuf::from(path))
}

fn read_bounded(file: File, limit: usize) -> io::Result<Vec<u8>> {
    let mut contents = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64 + 1).read_to_end(&mut contents)?;
    if contents.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "file grew beyond preview limit",
        ));
    }
    Ok(contents)
}

fn is_explicitly_unsupported(extension: &str) -> bool {
    matches!(
        extension,
        "svg"
            | "pdf"
            | "doc"
            | "docx"
            | "xls"
            | "xlsx"
            | "ppt"
            | "pptx"
            | "zip"
            | "7z"
            | "rar"
            | "mp3"
            | "wav"
            | "mp4"
            | "mov"
            | "avi"
    )
}

fn mark_ignored_entries(root: &Path, entries: &mut [DirectoryEntry]) {
    const MAX_INPUT_BYTES: usize = 64 * 1024;
    const MAX_PATHS: usize = 512;
    const MAX_OUTPUT_BYTES: usize = 128 * 1024;

    let mut batch = Vec::<(usize, Vec<u8>)>::new();
    let mut batch_bytes = 0;
    for index in 0..entries.len() {
        let path = {
            let Some(relative) = entries[index].path.strip_prefix(root).ok() else {
                continue;
            };
            let Some(relative) = relative.to_str() else {
                continue;
            };
            relative.as_bytes().to_vec()
        };
        if path.len() + 1 > MAX_INPUT_BYTES {
            continue;
        }
        if !batch.is_empty()
            && (batch.len() == MAX_PATHS || batch_bytes + path.len() + 1 > MAX_INPUT_BYTES)
        {
            apply_ignore_batch(root, entries, &batch, MAX_OUTPUT_BYTES);
            batch.clear();
            batch_bytes = 0;
        }
        batch_bytes += path.len() + 1;
        batch.push((index, path));
    }
    if !batch.is_empty() {
        apply_ignore_batch(root, entries, &batch, MAX_OUTPUT_BYTES);
    }
}

fn apply_ignore_batch(
    root: &Path,
    entries: &mut [DirectoryEntry],
    batch: &[(usize, Vec<u8>)],
    max_output_bytes: usize,
) {
    const TIMEOUT: Duration = Duration::from_millis(500);

    let Ok(mut child) = Command::new("git")
        .args(["check-ignore", "--no-index", "-z", "--stdin"])
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let Some(stdout) = child.stdout.take() else {
        terminate_child(&mut child);
        return;
    };
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .take(max_output_bytes as u64 + 1)
            .read_to_end(&mut output);
        (result, output)
    });
    let Some(mut stdin) = child.stdin.take() else {
        terminate_child(&mut child);
        let _ = reader.join();
        return;
    };
    let input = batch
        .iter()
        .map(|(_, path)| path.clone())
        .collect::<Vec<_>>();
    let writer = thread::spawn(move || {
        for path in input {
            stdin.write_all(&path)?;
            stdin.write_all(b"\0")?;
        }
        Ok::<(), io::Error>(())
    });

    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) | Err(_) => {
                terminate_child(&mut child);
                let _ = writer.join();
                let _ = reader.join();
                return;
            }
        }
    };
    let Ok(Ok(())) = writer.join() else {
        let _ = reader.join();
        return;
    };
    let Ok((Ok(_), output)) = reader.join() else {
        return;
    };
    let Some(status) = status else {
        return;
    };
    if output.len() > max_output_bytes || (!status.success() && status.code() != Some(1)) {
        return;
    }
    let ignored: std::collections::HashSet<&[u8]> = output.split(|byte| *byte == 0).collect();
    for (index, path) in batch {
        if ignored.contains(path.as_slice()) {
            entries[*index].ignored = true;
        }
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn is_raster_extension(extension: &str) -> bool {
    matches!(extension, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp")
}

fn has_matching_image_signature(extension: &str, data: &[u8]) -> bool {
    match extension {
        "png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
        "jpg" | "jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
        "gif" => data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a"),
        "webp" => data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP",
        "bmp" => data.starts_with(b"BM"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    fn write(root: &Path, relative: &str, contents: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[cfg(unix)]
    fn create_dir_symlink(target: impl AsRef<Path>, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_dir_symlink(target: impl AsRef<Path>, link: &Path) -> bool {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }

    #[cfg(unix)]
    fn create_file_symlink(target: impl AsRef<Path>, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_file_symlink(target: impl AsRef<Path>, link: &Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }

    fn can_create_dir_symlink(root: &Path, target: &Path) -> bool {
        let link = root.join("symlink-probe");
        let available = create_dir_symlink(target, &link);
        if available {
            let _ = fs::remove_file(link);
        }
        available
    }

    #[test]
    fn preview_rejects_parent_replacement_after_resolution_before_open() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        write(&root, "parent/file.txt", b"inside");
        write(&outside, "file.txt", b"outside secret");
        if !can_create_dir_symlink(&root, &outside) {
            return;
        }

        let result = read_preview_after_resolve(&root, Path::new("parent/file.txt"), || {
            let displaced = root.join("parent.original");
            fs::rename(root.join("parent"), displaced).unwrap();
            assert!(create_dir_symlink(&outside, &root.join("parent")));
        });
        assert!(
            result.is_err(),
            "must not return bytes from outside the root"
        );
    }

    #[test]
    fn image_reader_rejects_parent_replacement_after_resolution() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        write(&root, "parent/image.png", b"\x89PNG\r\n\x1a\ninside");
        write(&outside, "image.png", b"\x89PNG\r\n\x1a\noutside secret");
        if !can_create_dir_symlink(&root, &outside) {
            return;
        }

        let result = read_image_bytes_after_resolve(&root, Path::new("parent/image.png"), || {
            let displaced = root.join("parent.original");
            fs::rename(root.join("parent"), displaced).unwrap();
            assert!(create_dir_symlink(&outside, &root.join("parent")));
        });
        assert!(
            result.is_err(),
            "must not return image bytes from outside the root"
        );
    }

    #[test]
    fn directory_rejects_parent_replacement_after_resolution_before_enumeration() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        write(&root, "parent/inside.txt", b"inside");
        write(&outside, "secret.txt", b"outside secret");
        if !can_create_dir_symlink(&root, &outside) {
            return;
        }

        let result = read_directory_after_resolve(&root, Path::new("parent"), || {
            let displaced = root.join("parent.original");
            fs::rename(root.join("parent"), displaced).unwrap();
            assert!(create_dir_symlink(&outside, &root.join("parent")));
        });
        assert!(result.is_err(), "must not enumerate outside entries");
    }

    #[test]
    fn secure_image_reader_returns_bounded_verified_image_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let image = b"\x89PNG\r\n\x1a\ncontent";
        write(temp.path(), "image.png", image);
        assert_eq!(
            read_image_bytes(temp.path(), Path::new("image.png")).unwrap(),
            image
        );
        write(temp.path(), "fake.png", b"not png");
        assert!(read_image_bytes(temp.path(), Path::new("fake.png")).is_err());
    }

    #[test]
    fn directory_entry_limit_returns_explicit_error_at_the_limit() {
        let temp = tempfile::tempdir().unwrap();
        write(temp.path(), "one", b"1");
        write(temp.path(), "two", b"2");
        assert!(read_directory_with_limit(temp.path(), temp.path(), 1)
            .unwrap_err()
            .contains("exceeds the 1-entry limit"));
        assert_eq!(
            read_directory_with_limit(temp.path(), temp.path(), 2)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn directory_is_lazy_hides_git_and_sorts_directories_then_names() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir(root.join("z-dir")).unwrap();
        fs::create_dir(root.join("A-dir")).unwrap();
        write(root, "b.txt", b"b");
        write(root, "A.txt", b"a");
        write(root, ".env", b"secret");
        write(root, ".git/config", b"hidden");
        write(root, "z-dir/nested/file.txt", b"nested");

        let entries = read_directory(root, root).unwrap();
        let names: Vec<_> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, ["A-dir", "z-dir", ".env", "A.txt", "b.txt"]);
        assert!(entries[0].is_dir);
        assert_eq!(
            read_directory(root, &root.join("z-dir/nested")).unwrap()[0].name,
            "file.txt"
        );
    }

    #[test]
    fn directory_preserves_symlink_path_and_marks_symlink_without_following_tree() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        fs::create_dir_all(root.join("actual/child")).unwrap();
        if !create_dir_symlink("actual", &root.join("alias")) {
            return;
        }
        let entries = read_directory(&root, &root).unwrap();
        let alias = entries.iter().find(|entry| entry.name == "alias").unwrap();
        assert_eq!(alias.path, root.canonicalize().unwrap().join("alias"));
        assert!(alias.is_symlink && alias.is_dir);
        assert_eq!(read_directory(&root, &alias.path).unwrap()[0].name, "child");
    }

    #[test]
    fn resolve_rejects_parent_and_symlink_escape_but_allows_internal_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        write(&root, "inside.txt", b"inside");
        write(&outside, "secret.txt", b"secret");
        if !create_dir_symlink(&outside, &root.join("escape"))
            || !create_file_symlink(Path::new("inside.txt"), &root.join("alias.txt"))
        {
            return;
        }

        assert!(resolve_path(&root, Path::new("../outside/secret.txt")).is_err());
        assert!(resolve_path(&root, Path::new("escape/secret.txt")).is_err());
        let entries = read_directory(&root, &root).unwrap();
        let external_link = entries.iter().find(|entry| entry.name == "escape").unwrap();
        assert!(external_link.is_symlink && external_link.is_dir);
        assert!(read_directory(&root, &external_link.path).is_err());
        assert!(resolve_path(&root, Path::new(".git/config")).is_err());
        assert_eq!(
            resolve_path(&root, Path::new("alias.txt")).unwrap(),
            root.join("inside.txt").canonicalize().unwrap()
        );
    }

    #[test]
    fn directory_marks_gitignored_entries_with_one_safe_batch_query() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        assert!(std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()
            .unwrap()
            .success());
        write(root, ".gitignore", b"ignored.log\n");
        write(root, "ignored.log", b"ignored");
        write(root, "visible.log", b"visible");
        let entries = read_directory(root, root).unwrap();
        assert!(
            entries
                .iter()
                .find(|entry| entry.name == "ignored.log")
                .unwrap()
                .ignored
        );
        assert!(
            !entries
                .iter()
                .find(|entry| entry.name == "visible.log")
                .unwrap()
                .ignored
        );
    }

    #[test]
    fn preview_classifies_markdown_text_and_common_raster_images() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "README.MD", b"# Hello\n");
        write(root, "code.rs", "fn main() {}\n".as_bytes());
        write(
            root,
            "image.png",
            &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a],
        );
        write(root, "vector.svg", b"<svg/>");

        assert!(
            matches!(read_preview(root, &root.join("README.MD")).unwrap().content, PreviewContent::Markdown(s) if s == "# Hello\n")
        );
        assert!(
            matches!(read_preview(root, &root.join("code.rs")).unwrap().content, PreviewContent::Text(s) if s.starts_with("fn main"))
        );
        assert!(matches!(
            read_preview(root, &root.join("image.png")).unwrap().content,
            PreviewContent::Image
        ));
        assert!(matches!(
            read_preview(root, &root.join("vector.svg"))
                .unwrap()
                .content,
            PreviewContent::Unsupported(_)
        ));
    }

    #[test]
    fn preview_reports_binary_oversized_and_deleted_files_without_lossy_text() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "binary.txt", b"hello\0world");
        write(root, "invalid-utf8.txt", &[0xff, 0xfe]);
        write(root, "large.txt", &vec![b'x'; MAX_TEXT_BYTES + 1]);
        assert!(matches!(
            read_preview(root, &root.join("binary.txt"))
                .unwrap()
                .content,
            PreviewContent::Unsupported(_)
        ));
        assert!(matches!(
            read_preview(root, &root.join("invalid-utf8.txt"))
                .unwrap()
                .content,
            PreviewContent::Unsupported(_)
        ));
        assert!(matches!(
            read_preview(root, &root.join("large.txt")).unwrap().content,
            PreviewContent::Unsupported(_)
        ));
        assert!(read_preview(root, &root.join("deleted.txt")).is_err());
    }

    #[test]
    fn preview_rejects_non_regular_files_and_external_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let outside = temp.path().join("outside.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&outside, b"outside").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("outside.txt")).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&outside, root.join("outside.txt")).unwrap();
        fs::create_dir(root.join("folder")).unwrap();
        assert!(read_preview(&root, &root.join("outside.txt")).is_err());
        assert!(read_preview(&root, &root.join("folder")).is_err());
        #[cfg(unix)]
        {
            assert!(std::process::Command::new("mkfifo")
                .arg(root.join("pipe"))
                .status()
                .unwrap()
                .success());
            assert!(read_preview(&root, &root.join("pipe")).is_err());
        }
    }
}
