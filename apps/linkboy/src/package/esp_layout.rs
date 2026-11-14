//! Structure check for a vendor's merged ESP32-S3 image.

use super::{FirmwarePartKind, PackageError, VerifiedPackagePart};

/// The first byte of every ESP application and bootloader image header.
pub(crate) const ESP_IMAGE_MAGIC: u8 = 0xE9;

/// A merged image starts with the second-stage bootloader's image header at offset 0. This is
/// a structure check on the bytes; the manifest's hash is what vouches for them.
pub(super) fn validate_esp_merged(parts: &[VerifiedPackagePart]) -> Result<(), PackageError> {
    for part in parts {
        if part.declaration().kind == FirmwarePartKind::MergedImage
            && part.bytes().first() != Some(&ESP_IMAGE_MAGIC)
        {
            return Err(PackageError::InvalidField(format!(
                "merged image {} does not start with the ESP image magic 0xE9",
                part.path().display()
            )));
        }
    }
    Ok(())
}
