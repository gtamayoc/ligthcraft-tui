//! Removable drive detection, Win32 volume discovery, and DCIM directory scanning.
//!
//! Authoritative requirements:
//! - US-002: Automatic detection of removable drives and SD cards on Windows.
//! - R3 & FR-3: 100% Read-Only SD Protocol (zero write operations to SD media).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};

use super::IngestError;

/// Windows Drive Type Constants (Win32 API).
pub const DRIVE_UNKNOWN: u32 = 0;
pub const DRIVE_NO_ROOT_DIR: u32 = 1;
pub const DRIVE_REMOVABLE: u32 = 2;
pub const DRIVE_FIXED: u32 = 3;
pub const DRIVE_REMOTE: u32 = 4;
pub const DRIVE_CDROM: u32 = 5;
pub const DRIVE_RAMDISK: u32 = 6;

/// Photographic media category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaCategory {
    Raw,
    Jpeg,
    Video,
    Other,
}

impl MediaCategory {
    /// Classify a file extension (case-insensitive).
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_ascii_uppercase().as_str() {
            // Canon RAW
            "CR3" | "CR2" | "CRW"
            // Sony RAW
            | "ARW" | "SRF" | "SR2"
            // Nikon RAW
            | "NEF" | "NRW"
            // Adobe / Leica / Pentax RAW
            | "DNG"
            // Olympus / OM System RAW
            | "ORF" | "ORI"
            // Panasonic Lumix RAW
            | "RW2"
            // Fujifilm RAW
            | "RAF"
            // Pentax RAW
            | "PEF"
            // Hasselblad / Phase One / Others
            | "3FR" | "FFF" | "RWL" | "IIQ" => MediaCategory::Raw,

            // JPEG
            "JPG" | "JPEG" => MediaCategory::Jpeg,

            // Video
            "MP4" | "MOV" => MediaCategory::Video,

            // Non-media or unrecognized
            _ => MediaCategory::Other,
        }
    }

    /// Classify from file path.
    pub fn from_path(path: &Path) -> Self {
        path.extension()
            .and_then(|e| e.to_str())
            .map(Self::from_extension)
            .unwrap_or(MediaCategory::Other)
    }

    /// Returns `true` if the file is a photographic or video asset.
    pub fn is_media(&self) -> bool {
        matches!(self, MediaCategory::Raw | MediaCategory::Jpeg | MediaCategory::Video)
    }
}

/// Check if a file extension represents a camera RAW image.
pub fn is_raw_extension(ext: &str) -> bool {
    MediaCategory::from_extension(ext) == MediaCategory::Raw
}

/// Check if a file extension represents a standard JPEG image.
pub fn is_jpeg_extension(ext: &str) -> bool {
    MediaCategory::from_extension(ext) == MediaCategory::Jpeg
}

/// Check if a file extension represents a camera video recording.
pub fn is_video_extension(ext: &str) -> bool {
    MediaCategory::from_extension(ext) == MediaCategory::Video
}

/// Check if a file extension represents any supported photographic media.
pub fn is_media_extension(ext: &str) -> bool {
    MediaCategory::from_extension(ext).is_media()
}

/// Aggregated media counts and byte volumes across camera folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MediaSummary {
    pub raw_count: usize,
    pub jpeg_count: usize,
    pub video_count: usize,
    pub total_media_bytes: u64,
}

impl MediaSummary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total_count(&self) -> usize {
        self.raw_count + self.jpeg_count + self.video_count
    }

    pub fn is_empty(&self) -> bool {
        self.total_count() == 0
    }
}

/// Detailed media file item discovered during crawl.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaFileInfo {
    pub path: PathBuf,
    pub category: MediaCategory,
    pub size_bytes: u64,
}

// ============================================================================
// Core Crawler Functions (100% Read-Only)
// ============================================================================

/// Locate the DCIM directory inside a volume root (case-insensitive fallback).
pub fn find_dcim_dir(root: &Path) -> Option<PathBuf> {
    let direct = root.join("DCIM");
    if direct.is_dir() {
        return Some(direct);
    }

    // Case-insensitive search on root entries for non-Windows or odd mounts
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    if name.eq_ignore_ascii_case("dcim") {
                        return Some(entry.path());
                    }
                }
            }
        }
    }

    None
}

/// Enumerate all camera folders inside a DCIM directory (e.g. `100CANON`, `100MSDCF`).
/// Ignores hidden or system folders starting with `.` or `$`, or `"System Volume Information"`.
pub fn crawl_camera_folders(dcim_dir: &Path) -> Vec<PathBuf> {
    let mut camera_folders = Vec::new();
    if let Ok(entries) = fs::read_dir(dcim_dir) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() && !file_type.is_symlink() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('.')
                            && !name.starts_with('$')
                            && name != "System Volume Information"
                        {
                            camera_folders.push(entry.path());
                        }
                    }
                }
            }
        }
    }
    camera_folders.sort();
    camera_folders
}

/// Helper to identify hidden files, AppleDouble resource forks, or OS metadata.
#[inline]
pub fn is_dotfile_or_system_file(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with('$')
        || name == "System Volume Information"
        || name.eq_ignore_ascii_case("desktop.ini")
        || name.eq_ignore_ascii_case("thumbs.db")
}

/// Recursively crawl a directory to tally media assets into `MediaSummary`.
/// Strictly 100% read-only operations (`read_dir` and `metadata`).
fn crawl_dir_media(dir: &Path, summary: &mut MediaSummary) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                let path = entry.path();
                if ft.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        if is_dotfile_or_system_file(name) {
                            continue;
                        }
                    }
                    let cat = MediaCategory::from_path(&path);
                    if cat.is_media() {
                        let size = entry
                            .metadata()
                            .or_else(|_| fs::metadata(&path))
                            .map(|m| m.len())
                            .unwrap_or(0);
                        match cat {
                            MediaCategory::Raw => summary.raw_count += 1,
                            MediaCategory::Jpeg => summary.jpeg_count += 1,
                            MediaCategory::Video => summary.video_count += 1,
                            MediaCategory::Other => {}
                        }
                        summary.total_media_bytes += size;
                    }
                } else if ft.is_dir() && !ft.is_symlink() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !is_dotfile_or_system_file(name) {
                            crawl_dir_media(&path, summary);
                        }
                    }
                }
            }
        }
    }
}

/// Crawl a slice of camera folders and compute media counts and total byte volume.
pub fn crawl_media_summary(camera_folders: &[PathBuf]) -> MediaSummary {
    let mut summary = MediaSummary::default();
    for folder in camera_folders {
        crawl_dir_media(folder, &mut summary);
    }
    summary
}

/// Crawl volume root and return `(has_dcim, camera_folders, media_summary)`.
/// Guaranteed non-failing: returns empty results on missing DCIM or unreadable paths.
pub fn crawl_dcim(root: &Path) -> (bool, Vec<PathBuf>, MediaSummary) {
    if let Some(dcim_dir) = find_dcim_dir(root) {
        let camera_folders = crawl_camera_folders(&dcim_dir);
        let summary = crawl_media_summary(&camera_folders);
        (true, camera_folders, summary)
    } else {
        (false, Vec::new(), MediaSummary::default())
    }
}

/// Find the first photo path across camera folders (for EXIF camera fingerprinting).
///
/// Skips dotfiles/AppleDouble resource forks, recurses into camera subdirectories,
/// and skips 0-byte corrupt files so valid EXIF metadata can be extracted.
pub fn find_first_media_file(camera_folders: &[PathBuf]) -> Option<PathBuf> {
    for folder in camera_folders {
        if let Some(found) = find_first_photo_recursive(folder) {
            return Some(found);
        }
    }
    None
}

fn find_first_photo_recursive(dir: &Path) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(dir) {
        let mut subdirs = Vec::new();
        let mut entry_list: Vec<_> = entries.flatten().collect();
        entry_list.sort_by_key(|e| e.path());

        for entry in entry_list {
            let path = entry.path();
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        if is_dotfile_or_system_file(name) {
                            continue;
                        }
                    }
                    let cat = MediaCategory::from_path(&path);
                    if cat == MediaCategory::Raw || cat == MediaCategory::Jpeg {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        if size > 0 {
                            return Some(path);
                        }
                    }
                } else if file_type.is_dir() && !file_type.is_symlink() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !is_dotfile_or_system_file(name) {
                            subdirs.push(path);
                        }
                    }
                }
            }
        }

        for subdir in subdirs {
            if let Some(found) = find_first_photo_recursive(&subdir) {
                return Some(found);
            }
        }
    }
    None
}

/// Enumerate all media files across camera folders.
pub fn find_all_media_files(camera_folders: &[PathBuf]) -> Vec<MediaFileInfo> {
    let mut files = Vec::new();
    for folder in camera_folders {
        collect_media_files(folder, &mut files);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn collect_media_files(dir: &Path, out: &mut Vec<MediaFileInfo>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_file() {
                    if let Some(name) = entry.file_name().to_str() {
                        if is_dotfile_or_system_file(name) {
                            continue;
                        }
                    }
                    let category = MediaCategory::from_path(&path);
                    if category.is_media() {
                        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                        out.push(MediaFileInfo {
                            path,
                            category,
                            size_bytes: size,
                        });
                    }
                } else if file_type.is_dir() && !file_type.is_symlink() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !is_dotfile_or_system_file(name) {
                            collect_media_files(&path, out);
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================
// SdCardVolume & Trait Definition
// ============================================================================

/// Representation of a removable storage volume matching PROJECT.md and contracts.rs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SdCardVolume {
    pub drive_letter: char,
    pub volume_label: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub has_dcim: bool,
    pub camera_folders: Vec<PathBuf>,
}

impl SdCardVolume {
    /// Return the root directory path for this drive (e.g. `E:\`).
    pub fn root_path(&self) -> PathBuf {
        PathBuf::from(format!("{}:\\", self.drive_letter))
    }

    /// Check if the drive is a removable medium or camera card.
    pub fn is_removable(&self) -> bool {
        #[cfg(windows)]
        {
            let root = format!("{}:\\", self.drive_letter);
            let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
            let drive_type =
                unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(wide.as_ptr()) };
            if drive_type == DRIVE_REMOVABLE {
                return true;
            }
            if drive_type == DRIVE_FIXED || drive_type == DRIVE_CDROM {
                return self.has_dcim;
            }
        }
        // Fallback for mocks / synthetic volumes where drive letter is not mounted in OS
        self.has_dcim || (self.drive_letter >= 'D' && self.drive_letter <= 'Z')
    }

    /// Check if the filesystem is FAT, FAT32, or exFAT.
    pub fn is_fat_or_exfat(&self) -> bool {
        let fs = self.file_system.trim().to_ascii_uppercase();
        fs == "EXFAT" || fs == "FAT32" || fs == "FAT" || fs == "FAT16" || fs == "FAT12"
    }

    /// Compute real-time media summary across this volume's camera folders.
    pub fn media_summary(&self) -> MediaSummary {
        crawl_media_summary(&self.camera_folders)
    }

    /// Count of RAW files.
    pub fn raw_count(&self) -> usize {
        self.media_summary().raw_count
    }

    /// Count of JPEG files.
    pub fn jpeg_count(&self) -> usize {
        self.media_summary().jpeg_count
    }

    /// Count of Video files.
    pub fn video_count(&self) -> usize {
        self.media_summary().video_count
    }

    /// Total count of all media assets.
    pub fn media_count(&self) -> usize {
        self.media_summary().total_count()
    }

    /// Total bytes of media assets.
    pub fn total_media_bytes(&self) -> u64 {
        self.media_summary().total_media_bytes
    }

    /// True if the volume contains any media assets in camera folders.
    pub fn has_media(&self) -> bool {
        !self.media_summary().is_empty()
    }

    /// Path of the first photo for EXIF identification.
    pub fn first_photo(&self) -> Option<PathBuf> {
        find_first_media_file(&self.camera_folders)
    }
}

/// Trait abstracting volume scanning for removable drives.
pub trait VolumeScanner: Send + Sync {
    fn scan_volumes(&self) -> Result<Vec<SdCardVolume>, IngestError>;
}

/// Native Windows volume scanner calling Win32 kernel32 APIs.
#[derive(Debug, Clone, Default)]
pub struct WindowsVolumeScanner;

impl WindowsVolumeScanner {
    pub fn new() -> Self {
        Self
    }
}

impl VolumeScanner for WindowsVolumeScanner {
    fn scan_volumes(&self) -> Result<Vec<SdCardVolume>, IngestError> {
        #[cfg(windows)]
        {
            let mut volumes = Vec::new();
            let drives_mask =
                unsafe { windows_sys::Win32::Storage::FileSystem::GetLogicalDrives() };
            if drives_mask == 0 {
                return Ok(volumes);
            }

            for i in 0..26 {
                if (drives_mask & (1 << i)) == 0 {
                    continue;
                }

                let drive_letter = (b'A' + i as u8) as char;
                let root_str = format!("{}:\\", drive_letter);
                let root_wide: Vec<u16> =
                    root_str.encode_utf16().chain(std::iter::once(0)).collect();

                let drive_type = unsafe {
                    windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root_wide.as_ptr())
                };

                let root_path = PathBuf::from(&root_str);
                let dcim_opt = find_dcim_dir(&root_path);
                let has_dcim = dcim_opt.is_some();

                // Filter: Only DRIVE_REMOVABLE or drives with DCIM/ structure
                if drive_type != DRIVE_REMOVABLE && !has_dcim {
                    continue;
                }

                // Query free and total disk space
                let mut free_bytes_caller: u64 = 0;
                let mut total_bytes: u64 = 0;
                let mut free_bytes: u64 = 0;
                let space_ok = unsafe {
                    windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                        root_wide.as_ptr(),
                        &mut free_bytes_caller,
                        &mut total_bytes,
                        &mut free_bytes,
                    )
                };

                // Unready or media-less card reader: ignore safely without error
                if space_ok == 0 {
                    continue;
                }

                // Query volume label and file system
                let mut volume_name_buf = [0u16; 261];
                let mut fs_name_buf = [0u16; 261];
                let mut serial_number: u32 = 0;
                let mut max_component_len: u32 = 0;
                let mut fs_flags: u32 = 0;

                let vol_ok = unsafe {
                    windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW(
                        root_wide.as_ptr(),
                        volume_name_buf.as_mut_ptr(),
                        volume_name_buf.len() as u32,
                        &mut serial_number,
                        &mut max_component_len,
                        &mut fs_flags,
                        fs_name_buf.as_mut_ptr(),
                        fs_name_buf.len() as u32,
                    )
                };

                let (volume_label, file_system) = if vol_ok != 0 {
                    let vol_len = volume_name_buf
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(volume_name_buf.len());
                    let fs_len = fs_name_buf
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(fs_name_buf.len());
                    (
                        String::from_utf16_lossy(&volume_name_buf[..vol_len]),
                        String::from_utf16_lossy(&fs_name_buf[..fs_len]),
                    )
                } else {
                    (String::new(), String::new())
                };

                let camera_folders = if let Some(ref dcim_path) = dcim_opt {
                    crawl_camera_folders(dcim_path)
                } else {
                    Vec::new()
                };

                volumes.push(SdCardVolume {
                    drive_letter,
                    volume_label,
                    file_system,
                    total_bytes,
                    free_bytes,
                    has_dcim,
                    camera_folders,
                });
            }

            Ok(volumes)
        }
        #[cfg(not(windows))]
        {
            Ok(Vec::new())
        }
    }
}

/// In-memory mock scanner implementing `VolumeScanner` for unit and integration testing.
#[derive(Debug, Clone, Default)]
pub struct MockVolumeScanner {
    volumes: Arc<Mutex<Vec<SdCardVolume>>>,
    error_to_inject: Arc<Mutex<Option<IngestError>>>,
}

impl MockVolumeScanner {
    /// Create a new empty MockVolumeScanner.
    pub fn new() -> Self {
        Self {
            volumes: Arc::new(Mutex::new(Vec::new())),
            error_to_inject: Arc::new(Mutex::new(None)),
        }
    }

    /// Construct a MockVolumeScanner pre-populated with a vector of volumes.
    pub fn with_volumes(volumes: Vec<SdCardVolume>) -> Self {
        Self {
            volumes: Arc::new(Mutex::new(volumes)),
            error_to_inject: Arc::new(Mutex::new(None)),
        }
    }

    /// Add a simulated volume to the scanner.
    pub fn add_volume(&mut self, volume: SdCardVolume) {
        self.volumes.lock().unwrap().push(volume);
    }

    /// Remove all simulated volumes from the scanner.
    pub fn clear(&mut self) {
        self.volumes.lock().unwrap().clear();
    }

    /// Inject a simulated error to be returned on `scan_volumes`.
    pub fn set_error(&mut self, error: Option<IngestError>) {
        *self.error_to_inject.lock().unwrap() = error;
    }
}

impl VolumeScanner for MockVolumeScanner {
    fn scan_volumes(&self) -> Result<Vec<SdCardVolume>, IngestError> {
        if let Some(ref err) = *self.error_to_inject.lock().unwrap() {
            return Err(err.clone());
        }
        Ok(self.volumes.lock().unwrap().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_mock_volume_scanner_lifecycle() {
        let mut scanner = MockVolumeScanner::new();
        assert_eq!(scanner.scan_volumes().unwrap().len(), 0);

        let vol = SdCardVolume {
            drive_letter: 'E',
            volume_label: "EOS_DIGITAL".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 128 * 1024 * 1024 * 1024,
            free_bytes: 64 * 1024 * 1024 * 1024,
            has_dcim: true,
            camera_folders: vec![PathBuf::from(r"E:\DCIM\100CANON")],
        };

        scanner.add_volume(vol.clone());
        let vols = scanner.scan_volumes().unwrap();
        assert_eq!(vols.len(), 1);
        assert_eq!(vols[0].drive_letter, 'E');
        assert_eq!(vols[0].volume_label, "EOS_DIGITAL");
        assert_eq!(vols[0].file_system, "exFAT");
        assert!(vols[0].has_dcim);

        scanner.clear();
        assert_eq!(scanner.scan_volumes().unwrap().len(), 0);
    }

    #[test]
    fn test_mock_volume_scanner_error_injection() {
        let mut scanner = MockVolumeScanner::new();
        scanner.set_error(Some(IngestError::ScanError("Simulated bus error".to_string())));

        let res = scanner.scan_volumes();
        assert!(res.is_err());
        assert_eq!(
            res.unwrap_err(),
            IngestError::ScanError("Simulated bus error".to_string())
        );

        scanner.set_error(None);
        assert!(scanner.scan_volumes().is_ok());
    }

    #[test]
    fn test_dcim_crawler_camera_folders_and_media_categorization() {
        let dir = tempdir().expect("create tempdir");
        let root = dir.path();

        // Initially no DCIM
        assert!(find_dcim_dir(root).is_none());
        let (has_dcim, folders, summary) = crawl_dcim(root);
        assert!(!has_dcim);
        assert!(folders.is_empty());
        assert_eq!(summary.total_count(), 0);

        // Create DCIM directory and camera subdirectories
        let dcim = root.join("DCIM");
        fs::create_dir_all(&dcim).expect("create DCIM");
        let cam_canon = dcim.join("100CANON");
        let cam_sony = dcim.join("100MSDCF");
        let hidden_dir = dcim.join(".Spotlight-V100");
        let sys_dir = dcim.join("System Volume Information");

        fs::create_dir_all(&cam_canon).expect("create 100CANON");
        fs::create_dir_all(&cam_sony).expect("create 100MSDCF");
        fs::create_dir_all(&hidden_dir).expect("create hidden");
        fs::create_dir_all(&sys_dir).expect("create sys");

        // Verify camera folders are enumerated and hidden ones skipped
        let camera_folders = crawl_camera_folders(&dcim);
        assert_eq!(camera_folders.len(), 2);
        assert!(camera_folders.contains(&cam_canon));
        assert!(camera_folders.contains(&cam_sony));

        // Populate media files in 100CANON
        let raw_file = cam_canon.join("IMG_0001.CR3");
        fs::write(&raw_file, vec![1u8; 1024]).expect("write raw");

        let jpg_file = cam_canon.join("IMG_0002.jpg");
        fs::write(&jpg_file, vec![2u8; 2048]).expect("write jpg");

        // Populate video file and non-media in 100MSDCF
        let vid_file = cam_sony.join("C0001.MP4");
        fs::write(&vid_file, vec![3u8; 4096]).expect("write mp4");

        let non_media = cam_sony.join("INDEX.CTG");
        fs::write(&non_media, vec![4u8; 512]).expect("write ctg");

        let summary = crawl_media_summary(&camera_folders);
        assert_eq!(summary.raw_count, 1);
        assert_eq!(summary.jpeg_count, 1);
        assert_eq!(summary.video_count, 1);
        assert_eq!(summary.total_count(), 3);
        assert_eq!(summary.total_media_bytes, 1024 + 2048 + 4096);

        // Test SdCardVolume methods
        let vol = SdCardVolume {
            drive_letter: 'E',
            volume_label: "CANON_EOS".to_string(),
            file_system: "exFAT".to_string(),
            total_bytes: 128 * 1024 * 1024 * 1024,
            free_bytes: 100 * 1024 * 1024 * 1024,
            has_dcim: true,
            camera_folders: camera_folders.clone(),
        };

        assert_eq!(vol.raw_count(), 1);
        assert_eq!(vol.jpeg_count(), 1);
        assert_eq!(vol.video_count(), 1);
        assert_eq!(vol.media_count(), 3);
        assert_eq!(vol.total_media_bytes(), 1024 + 2048 + 4096);
        assert!(vol.has_media());
        assert!(vol.is_fat_or_exfat());
        assert!(vol.is_removable());
        assert!(vol.first_photo().is_some());
    }

    #[test]
    fn test_media_category_classification() {
        assert_eq!(MediaCategory::from_extension("cr3"), MediaCategory::Raw);
        assert_eq!(MediaCategory::from_extension("CR2"), MediaCategory::Raw);
        assert_eq!(MediaCategory::from_extension("arw"), MediaCategory::Raw);
        assert_eq!(MediaCategory::from_extension("NEF"), MediaCategory::Raw);
        assert_eq!(MediaCategory::from_extension("dng"), MediaCategory::Raw);
        assert_eq!(MediaCategory::from_extension("orf"), MediaCategory::Raw);
        assert_eq!(MediaCategory::from_extension("rw2"), MediaCategory::Raw);
        assert_eq!(MediaCategory::from_extension("raf"), MediaCategory::Raw);

        assert_eq!(MediaCategory::from_extension("jpg"), MediaCategory::Jpeg);
        assert_eq!(MediaCategory::from_extension("JPEG"), MediaCategory::Jpeg);

        assert_eq!(MediaCategory::from_extension("mp4"), MediaCategory::Video);
        assert_eq!(MediaCategory::from_extension("mov"), MediaCategory::Video);

        assert_eq!(MediaCategory::from_extension("txt"), MediaCategory::Other);
        assert_eq!(MediaCategory::from_extension("xmp"), MediaCategory::Other);
        assert_eq!(MediaCategory::from_extension("thm"), MediaCategory::Other);
    }

    #[test]
    fn test_windows_volume_scanner_smoke() {
        let scanner = WindowsVolumeScanner::new();
        let res = scanner.scan_volumes();
        assert!(res.is_ok());
    }
}
