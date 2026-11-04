//! Recording/screenshot library: one SQLite table holding the full MediaMetadata JSON
//! plus indexed columns (title, path, created_at, size) for queries and edits.

use crate::util::{sanitize_file_name, unique_path};
use capture_core::*;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const SCHEMA_VERSION: i32 = 1;

pub struct Library {
    conn: Connection,
}

fn db_err(e: rusqlite::Error) -> CaptureError {
    CaptureError::new(CaptureErrorCode::IoError, format!("library database: {e}"))
}

fn json_err(e: serde_json::Error) -> CaptureError {
    CaptureError::internal(format!("library metadata: {e}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageTotals {
    pub total_bytes: u64,
    pub video_count: u64,
    pub screenshot_count: u64,
}

impl Library {
    pub fn open(path: &Path) -> CaptureResult<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> CaptureResult<Self> {
        Self::init(Connection::open_in_memory().map_err(db_err)?)
    }

    fn init(conn: Connection) -> CaptureResult<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS media (
               id TEXT PRIMARY KEY NOT NULL,
               kind TEXT NOT NULL,
               path TEXT NOT NULL UNIQUE,
               file_name TEXT NOT NULL,
               title TEXT NOT NULL DEFAULT '',
               created_at TEXT NOT NULL,
               size_bytes INTEGER NOT NULL,
               thumbnail_path TEXT,
               metadata TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS media_created_at ON media(created_at DESC);",
        )
        .map_err(db_err)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION).map_err(db_err)?;
        Ok(Self { conn })
    }

    /// Inserts or replaces an item. An existing row for the same file path (e.g. a
    /// recovered recording) is replaced so each file appears once.
    pub fn upsert(&mut self, m: &MediaMetadata) -> CaptureResult<MediaMetadata> {
        let json = serde_json::to_string(m).map_err(json_err)?;
        let tx = self.conn.transaction().map_err(db_err)?;
        tx.execute("DELETE FROM media WHERE path = ?1 AND id <> ?2", params![m.path, m.id]).map_err(db_err)?;
        tx.execute(
            "INSERT INTO media (id, kind, path, file_name, title, created_at, size_bytes, thumbnail_path, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, path=excluded.path, file_name=excluded.file_name,
               title=excluded.title, created_at=excluded.created_at, size_bytes=excluded.size_bytes,
               thumbnail_path=excluded.thumbnail_path, metadata=excluded.metadata",
            params![
                m.id,
                kind_str(m.kind),
                m.path,
                m.file_name,
                m.title,
                m.created_at,
                m.size_bytes as i64,
                m.thumbnail_path,
                json
            ],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(m.clone())
    }

    pub fn get(&self, id: &str) -> CaptureResult<Option<MediaMetadata>> {
        self.conn
            .query_row("SELECT metadata FROM media WHERE id = ?1", params![id], |r| r.get::<_, String>(0))
            .optional()
            .map_err(db_err)?
            .map(|j| serde_json::from_str(&j).map_err(json_err))
            .transpose()
    }

    fn require(&self, id: &str) -> CaptureResult<MediaMetadata> {
        self.get(id)?.ok_or_else(|| CaptureError::new(CaptureErrorCode::SourceNotFound, format!("library item '{id}' not found")))
    }

    pub fn contains_path(&self, path: &str) -> CaptureResult<bool> {
        self.conn.query_row("SELECT COUNT(*) FROM media WHERE path = ?1", params![path], |r| r.get::<_, i64>(0)).map(|n| n > 0).map_err(db_err)
    }

    /// Newest first. Rows with unreadable metadata are skipped (and logged).
    pub fn list(&self) -> CaptureResult<Vec<MediaMetadata>> {
        let mut stmt = self.conn.prepare("SELECT id, metadata FROM media ORDER BY created_at DESC, rowid DESC").map_err(db_err)?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).map_err(db_err)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, json) = row.map_err(db_err)?;
            match serde_json::from_str::<MediaMetadata>(&json) {
                Ok(m) => out.push(m),
                Err(e) => log::warn!("library: skipping unreadable row {id}: {e}"),
            }
        }
        Ok(out)
    }

    /// Sets the title; with `rename_file`, also renames the file on disk to the
    /// sanitized title (keeping the extension; " (2)" suffix on collisions).
    pub fn rename(&mut self, id: &str, title: &str, rename_file: bool) -> CaptureResult<MediaMetadata> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 120 {
            return Err(CaptureError::invalid_config("title must be 1..120 characters"));
        }
        let mut m = self.require(id)?;
        m.title = title.to_string();
        if rename_file {
            let current = PathBuf::from(&m.path);
            let dir = current.parent().ok_or_else(|| CaptureError::io("file has no parent directory"))?;
            let ext = current.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
            let target = unique_path(dir, &sanitize_file_name(title), &ext, Some(&current));
            if target != current {
                if !current.exists() {
                    return Err(CaptureError::new(CaptureErrorCode::SourceNotFound, format!("{} no longer exists", current.display())));
                }
                std::fs::rename(&current, &target)?;
                m.path = target.to_string_lossy().to_string();
                m.file_name = target.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            }
        }
        self.upsert(&m)
    }

    /// Removes the row; with `delete_file`, also deletes the media file and its thumbnail.
    pub fn delete(&mut self, id: &str, delete_file: bool) -> CaptureResult<MediaMetadata> {
        let m = self.require(id)?;
        if delete_file {
            match std::fs::remove_file(&m.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            if let Some(t) = &m.thumbnail_path {
                if t != &m.path {
                    let _ = std::fs::remove_file(t);
                }
            }
        }
        self.conn.execute("DELETE FROM media WHERE id = ?1", params![id]).map_err(db_err)?;
        Ok(m)
    }

    /// Copies the file to `destination` (overwriting, as confirmed by the save dialog).
    pub fn export(&self, id: &str, destination: &Path) -> CaptureResult<PathBuf> {
        let m = self.require(id)?;
        let src = PathBuf::from(&m.path);
        if !src.is_file() {
            return Err(CaptureError::new(CaptureErrorCode::SourceNotFound, format!("{} no longer exists", src.display())));
        }
        // The destination comes from the webview: keep it to a plain, same-type file path so a
        // compromised UI cannot overwrite dotfiles, launchers or other library recordings.
        if !destination.is_absolute() || destination.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(CaptureError::invalid_config("export destination must be an absolute path without '..'"));
        }
        let ext = |p: &Path| p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
        if ext(destination).is_none() || ext(destination) != ext(&src) {
            return Err(CaptureError::invalid_config("export destination must keep the file extension of the recording"));
        }
        if destination.is_dir() || std::fs::symlink_metadata(destination).map(|m| m.file_type().is_symlink()).unwrap_or(false) {
            return Err(CaptureError::invalid_config("export destination must be a regular file path"));
        }
        if self.contains_path(&destination.to_string_lossy())? {
            return Err(CaptureError::invalid_config("export destination is another library item"));
        }
        let same = match (std::fs::canonicalize(&src), destination.parent().and_then(|p| std::fs::canonicalize(p).ok())) {
            (Ok(a), Some(dir)) => destination.file_name().map(|f| dir.join(f) == a).unwrap_or(false),
            _ => false,
        };
        if same {
            return Err(CaptureError::invalid_config("export destination is the original file"));
        }
        if let Some(dir) = destination.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)?;
            }
        }
        std::fs::copy(&src, destination)?;
        Ok(destination.to_path_buf())
    }

    /// Replaces technical metadata with a fresh inspection while keeping the library
    /// identity (id, title, creation time, thumbnail, source/platform).
    pub fn apply_inspection(&mut self, id: &str, inspected: MediaMetadata) -> CaptureResult<MediaMetadata> {
        let old = self.require(id)?;
        let merged = MediaMetadata {
            id: old.id.clone(),
            title: old.title.clone(),
            created_at: old.created_at.clone(),
            thumbnail_path: inspected.thumbnail_path.clone().or(old.thumbnail_path.clone()),
            source_name: inspected.source_name.clone().or(old.source_name.clone()),
            platform: inspected.platform.clone().or(old.platform.clone()),
            path: old.path.clone(),
            file_name: old.file_name.clone(),
            ..inspected
        };
        self.upsert(&merged)
    }

    /// Totals over existing files (current on-disk size when available).
    pub fn storage_totals(&self) -> CaptureResult<StorageTotals> {
        let mut t = StorageTotals::default();
        for m in self.list()? {
            let size = std::fs::metadata(&m.path).map(|md| md.len()).unwrap_or(0);
            if size == 0 && !Path::new(&m.path).exists() {
                continue;
            }
            t.total_bytes += size;
            match m.kind {
                MediaKind::Video => t.video_count += 1,
                MediaKind::Screenshot => t.screenshot_count += 1,
            }
        }
        Ok(t)
    }
}

fn kind_str(k: MediaKind) -> &'static str {
    match k {
        MediaKind::Video => "video",
        MediaKind::Screenshot => "screenshot",
    }
}

/// Marks items whose file disappeared as invalid (for display only; not persisted).
pub fn annotate_missing(mut items: Vec<MediaMetadata>) -> Vec<MediaMetadata> {
    for m in items.iter_mut() {
        if !Path::new(&m.path).exists() {
            m.validation = Some(MediaValidation {
                valid: false,
                checked_at: crate::util::now_iso(),
                tool: "captureone".into(),
                issues: vec![ValidationIssue { severity: IssueSeverity::Error, code: "FILE_MISSING".into(), message: "The file was moved or deleted outside CaptureOne.".into() }],
            });
        }
    }
    items
}

/// Free space on the volume holding `dir` (walks up to the nearest existing ancestor).
pub fn free_disk_bytes(dir: &Path) -> u64 {
    let mut p = Some(dir);
    while let Some(cur) = p {
        if cur.exists() {
            return fs4::available_space(cur).unwrap_or(0);
        }
        p = cur.parent();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(dir: &Path, id: &str, name: &str, kind: MediaKind, created: &str) -> MediaMetadata {
        let path = dir.join(name);
        std::fs::write(&path, vec![7u8; 1000]).unwrap();
        MediaMetadata {
            id: id.into(),
            kind,
            path: path.to_string_lossy().to_string(),
            file_name: name.into(),
            title: String::new(),
            created_at: created.into(),
            size_bytes: 1000,
            width: 1920,
            height: 1080,
            duration_ms: if kind == MediaKind::Video { Some(5000.0) } else { None },
            fps: None,
            video_codec: if kind == MediaKind::Video { Some(VideoCodec::H264) } else { None },
            video_bitrate_kbps: None,
            audio_tracks: vec![],
            image_format: if kind == MediaKind::Screenshot { Some(ImageFormat::Png) } else { None },
            container: if kind == MediaKind::Video { "mp4".into() } else { "png".into() },
            source_name: Some("Display 1".into()),
            platform: Some("linux".into()),
            validation: Some(MediaValidation { valid: true, checked_at: "2026-11-04T00:00:00Z".into(), tool: "ffprobe".into(), issues: vec![] }),
            thumbnail_path: None,
        }
    }

    #[test]
    fn crud_round_trip_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("data/library.sqlite3");
        {
            let mut lib = Library::open(&db).unwrap();
            lib.upsert(&sample(dir.path(), "a", "a.mp4", MediaKind::Video, "2026-11-04T10:00:00Z")).unwrap();
            lib.upsert(&sample(dir.path(), "b", "b.png", MediaKind::Screenshot, "2026-11-04T11:00:00Z")).unwrap();
            assert_eq!(lib.list().unwrap().iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["b", "a"]);
        }
        let lib = Library::open(&db).unwrap();
        assert_eq!(lib.list().unwrap().len(), 2);
        assert_eq!(lib.get("a").unwrap().unwrap().source_name.as_deref(), Some("Display 1"));
        assert!(lib.get("zzz").unwrap().is_none());
        assert!(lib.contains_path(&dir.path().join("a.mp4").to_string_lossy()).unwrap());
    }

    #[test]
    fn upsert_dedupes_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        let a = sample(dir.path(), "a", "rec.mp4", MediaKind::Video, "2026-11-04T10:00:00Z");
        lib.upsert(&a).unwrap();
        let recovered = MediaMetadata { id: "recovered".into(), ..a.clone() };
        lib.upsert(&recovered).unwrap();
        let ids: Vec<_> = lib.list().unwrap().into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["recovered"]);
    }

    #[test]
    fn rename_title_only_and_with_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert(&sample(dir.path(), "a", "a.mp4", MediaKind::Video, "2026-11-04T10:00:00Z")).unwrap();
        std::fs::write(dir.path().join("Sprint demo_ final.mp4"), b"taken").unwrap();

        let m = lib.rename("a", "  Sprint demo  ", false).unwrap();
        assert_eq!(m.title, "Sprint demo");
        assert!(m.path.ends_with("a.mp4"));

        let m = lib.rename("a", "Sprint demo: final", true).unwrap();
        assert_eq!(m.file_name, "Sprint demo_ final (2).mp4");
        assert!(Path::new(&m.path).exists());
        assert!(!dir.path().join("a.mp4").exists());
        assert_eq!(lib.get("a").unwrap().unwrap().path, m.path);

        // Renaming to the same name keeps the file where it is.
        let again = lib.rename("a", "Sprint demo: final", true).unwrap();
        assert_eq!(again.path, m.path);

        // Titles can never steer the file outside its folder.
        let m = lib.rename("a", "../../etc/cron.d/evil", true).unwrap();
        assert_eq!(Path::new(&m.path).parent().unwrap(), dir.path());
        assert!(!m.file_name.contains('/') && m.file_name.ends_with(".mp4"));
        let m = lib.rename("a", "..", true).unwrap();
        assert_eq!(Path::new(&m.path).parent().unwrap(), dir.path());
        assert_eq!(lib.rename("a", "   ", false).unwrap_err().code, CaptureErrorCode::InvalidConfig);
        assert_eq!(lib.rename("missing", "x", false).unwrap_err().code, CaptureErrorCode::SourceNotFound);
    }

    #[test]
    fn delete_row_only_or_with_file_and_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        let mut a = sample(dir.path(), "a", "a.mp4", MediaKind::Video, "2026-11-04T10:00:00Z");
        let thumb = dir.path().join("a.jpg");
        std::fs::write(&thumb, b"jpg").unwrap();
        a.thumbnail_path = Some(thumb.to_string_lossy().to_string());
        lib.upsert(&a).unwrap();
        lib.upsert(&sample(dir.path(), "b", "b.png", MediaKind::Screenshot, "2026-11-04T11:00:00Z")).unwrap();

        lib.delete("b", false).unwrap();
        assert!(dir.path().join("b.png").exists());
        lib.delete("a", true).unwrap();
        assert!(!dir.path().join("a.mp4").exists());
        assert!(!thumb.exists());
        assert!(lib.list().unwrap().is_empty());
        assert_eq!(lib.delete("a", true).unwrap_err().code, CaptureErrorCode::SourceNotFound);
    }

    #[test]
    fn export_copies_and_refuses_self_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert(&sample(dir.path(), "a", "a.mp4", MediaKind::Video, "2026-11-04T10:00:00Z")).unwrap();
        let dest = dir.path().join("exports/copy.mp4");
        assert_eq!(lib.export("a", &dest).unwrap(), dest);
        assert_eq!(std::fs::read(&dest).unwrap().len(), 1000);
        assert_eq!(lib.export("a", &dir.path().join("a.mp4")).unwrap_err().code, CaptureErrorCode::InvalidConfig);
        // Hardening: relative, traversal, wrong extension, directory and other-library-item targets.
        for bad in [PathBuf::from("rel/copy.mp4"), dir.path().join("../escape.mp4"), dir.path().join(".bashrc"), dir.path().join("copy.desktop"), dir.path().join("exports")] {
            assert_eq!(lib.export("a", &bad).unwrap_err().code, CaptureErrorCode::InvalidConfig, "{bad:?}");
        }
        lib.upsert(&sample(dir.path(), "b", "b.mp4", MediaKind::Video, "2026-11-04T11:00:00Z")).unwrap();
        assert_eq!(lib.export("a", &dir.path().join("b.mp4")).unwrap_err().code, CaptureErrorCode::InvalidConfig);
        std::fs::remove_file(dir.path().join("a.mp4")).unwrap();
        assert_eq!(lib.export("a", &dest).unwrap_err().code, CaptureErrorCode::SourceNotFound);
    }

    #[test]
    fn inspection_keeps_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        let mut a = sample(dir.path(), "a", "a.mp4", MediaKind::Video, "2026-11-04T10:00:00Z");
        a.title = "Keep me".into();
        a.thumbnail_path = Some("/t/a.jpg".into());
        lib.upsert(&a).unwrap();
        let inspected = MediaMetadata {
            id: "fresh-uuid".into(),
            title: String::new(),
            created_at: "1999-01-01T00:00:00Z".into(),
            thumbnail_path: None,
            source_name: None,
            duration_ms: Some(4321.0),
            validation: Some(MediaValidation { valid: false, checked_at: "now".into(), tool: "ffprobe".into(), issues: vec![ValidationIssue { severity: IssueSeverity::Error, code: "TRUNCATED".into(), message: "moov missing".into() }] }),
            ..a.clone()
        };
        let m = lib.apply_inspection("a", inspected).unwrap();
        assert_eq!((m.id.as_str(), m.title.as_str(), m.created_at.as_str()), ("a", "Keep me", "2026-11-04T10:00:00Z"));
        assert_eq!(m.thumbnail_path.as_deref(), Some("/t/a.jpg"));
        assert_eq!(m.source_name.as_deref(), Some("Display 1"));
        assert_eq!(m.duration_ms, Some(4321.0));
        assert!(!m.validation.unwrap().valid);
        assert_eq!(lib.list().unwrap().len(), 1);
    }

    #[test]
    fn storage_totals_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::open_in_memory().unwrap();
        lib.upsert(&sample(dir.path(), "a", "a.mp4", MediaKind::Video, "2026-11-04T10:00:00Z")).unwrap();
        lib.upsert(&sample(dir.path(), "b", "b.png", MediaKind::Screenshot, "2026-11-04T11:00:00Z")).unwrap();
        lib.upsert(&sample(dir.path(), "c", "c.mp4", MediaKind::Video, "2026-11-04T12:00:00Z")).unwrap();
        std::fs::remove_file(dir.path().join("c.mp4")).unwrap();
        assert_eq!(lib.storage_totals().unwrap(), StorageTotals { total_bytes: 2000, video_count: 1, screenshot_count: 1 });
        let annotated = annotate_missing(lib.list().unwrap());
        let c = annotated.iter().find(|m| m.id == "c").unwrap();
        assert_eq!(c.validation.as_ref().unwrap().issues[0].code, "FILE_MISSING");
        assert!(annotated.iter().find(|m| m.id == "a").unwrap().validation.as_ref().unwrap().valid);
        assert!(free_disk_bytes(&dir.path().join("does/not/exist")) > 0);
    }
}
