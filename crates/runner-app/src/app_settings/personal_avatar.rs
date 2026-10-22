use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context as _, Result};
use runner_core::protocol::model::{normalize_avatar_color, AvatarStyle};
use serde::{Deserialize, Serialize};

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 2048;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PersonalAvatarSettings {
    pub style: AvatarStyle,
    pub image_path: Option<String>,
    pub color: Option<String>,
}

impl PersonalAvatarSettings {
    pub(super) fn normalize(&mut self) {
        self.color = self.color.as_deref().and_then(normalize_avatar_color);
        self.image_path = self.image_path.take().and_then(|path| {
            let path = path.trim();
            (!path.is_empty()).then(|| path.to_owned())
        });
    }
}

pub fn import_personal_avatar(source: &Path, app_data_dir: &Path) -> Result<PathBuf> {
    let metadata = fs::metadata(source).context("read avatar file metadata")?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
        bail!("Avatar must be a PNG file no larger than 8 MiB");
    }
    let mut bytes = Vec::new();
    File::open(source)
        .context("open avatar PNG")?
        .take((MAX_IMAGE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("read avatar PNG")?;
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        bail!("Avatar PNG must be between 1 byte and 8 MiB");
    }
    let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes))
        .read_info()
        .context("Avatar must be a valid PNG")?;
    let info = reader.info();
    if info.width == 0
        || info.height == 0
        || info.width > MAX_IMAGE_DIMENSION
        || info.height > MAX_IMAGE_DIMENSION
        || u64::from(info.width) * u64::from(info.height) > 4 * 1024 * 1024
    {
        bail!("Avatar PNG dimensions must not exceed 2048 × 2048");
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= MAX_IMAGE_BYTES)
        .context("Avatar PNG expands beyond the safe size limit")?;
    reader
        .next_frame(&mut vec![0; size])
        .context("Avatar PNG is incomplete")?;

    // Personal settings are not role references: role-preview cleanup must not own these files.
    let directory = app_data_dir.join("personal-avatars");
    fs::create_dir_all(&directory).context("create personal avatar directory")?;
    let mut image = tempfile::Builder::new()
        .prefix("avatar-")
        .suffix(".png")
        .tempfile_in(&directory)
        .context("create personal avatar copy")?;
    image
        .write_all(&bytes)
        .context("write personal avatar copy")?;
    let (_, path) = image.keep().context("keep personal avatar copy")?;
    Ok(path)
}

pub fn discard_personal_avatar(path: &Path, app_data_dir: &Path) -> Result<bool> {
    if !fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file()) {
        return Ok(false);
    }
    let Ok(root) = app_data_dir.canonicalize() else {
        return Ok(false);
    };
    let directory = root.join("personal-avatars");
    let Ok(actual) = path.canonicalize() else {
        return Ok(false);
    };
    if actual.parent() != Some(directory.as_path())
        || !actual
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("avatar-") && name.ends_with(".png"))
    {
        return Ok(false);
    }
    fs::remove_file(path).context("remove unused personal avatar copy")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![255; (width * height * 4) as usize])
                .unwrap();
        }
        bytes
    }

    #[test]
    fn personal_avatar_import_owns_a_unique_copy_without_changing_the_source() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let source = root.path().join("original.png");
        let bytes = png_bytes(2, 2);
        std::fs::write(&source, &bytes).unwrap();
        let first = import_personal_avatar(&source, &data).unwrap();
        let second = import_personal_avatar(&source, &data).unwrap();
        assert_ne!(first, second);
        assert_eq!(first.parent().unwrap(), data.join("personal-avatars"));
        assert!(!first.starts_with(data.join("role-avatars")));
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        std::fs::remove_file(&source).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), bytes);
        assert_eq!(std::fs::read(&second).unwrap(), bytes);
    }

    #[test]
    fn personal_avatar_import_rejects_invalid_truncated_and_oversized_images() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("original.png");
        let data = root.path().join("data");
        let mut incomplete_frame = png_bytes(2, 2);
        incomplete_frame.truncate(incomplete_frame.len() - 18);
        for bytes in [
            b"not a PNG".to_vec(),
            png_bytes(2, 2)[..33].to_vec(),
            incomplete_frame,
            vec![0; 8 * 1024 * 1024 + 1],
            png_bytes(2049, 1),
        ] {
            std::fs::write(&source, &bytes).unwrap();
            assert!(import_personal_avatar(&source, &data).is_err());
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            assert!(!data.exists());
        }
    }

    #[test]
    fn personal_avatar_cleanup_only_removes_owned_copies() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        let source = root.path().join("avatar-source.png");
        fs::write(&source, png_bytes(1, 1)).unwrap();
        let owned = import_personal_avatar(&source, &data).unwrap();
        assert!(!discard_personal_avatar(&source, &data).unwrap());
        assert!(source.is_file());
        let role_dir = data.join("role-avatars");
        fs::create_dir(&role_dir).unwrap();
        let role_image = role_dir.join("avatar-role.png");
        fs::copy(&source, &role_image).unwrap();
        assert!(!discard_personal_avatar(&role_image, &data).unwrap());
        assert!(role_image.is_file());
        #[cfg(unix)]
        {
            let source_link = root.path().join("avatar-source-link.png");
            std::os::unix::fs::symlink(&owned, &source_link).unwrap();
            assert!(!discard_personal_avatar(&source_link, &data).unwrap());
            assert!(source_link.symlink_metadata().is_ok());
        }
        assert!(discard_personal_avatar(&owned, &data).unwrap());
        assert!(!owned.exists());
        assert!(source.is_file());
    }

    #[test]
    fn personal_avatar_import_failure_never_deletes_the_original() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("original.png");
        std::fs::write(&source, png_bytes(1, 1)).unwrap();
        let blocked = root.path().join("not-a-directory");
        std::fs::write(&blocked, b"keep").unwrap();
        assert!(import_personal_avatar(&source, &blocked).is_err());
        assert!(source.is_file());
        assert_eq!(std::fs::read(blocked).unwrap(), b"keep");
    }
}
