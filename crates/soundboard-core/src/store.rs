//! The sound set library on disk: one `<set id>.soundset` folder per set, holding `Set.json`,
//! a `Sounds/` folder with a private copy of every audio file and a `Waveforms/` cache.
//! Sets travel between machines as a single `.soundset` zip file with the same layout.

use crate::model::{SoundSet, Tile};
use crate::waveform::{self, Peak};
use crate::Uuid;
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

pub const PACKAGE_EXTENSION: &str = "soundset";
const SET_FILE: &str = "Set.json";
const FOLDERS: [&str; 2] = ["Sounds", "Waveforms"];
/// An imported set larger than this once unpacked is refused rather than filling the disk.
const MAX_IMPORT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub struct StoreError(pub String);

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError(e.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError(format!("fichier de set illisible ({e})"))
    }
}

impl From<zip::result::ZipError> for StoreError {
    fn from(e: zip::result::ZipError) -> Self {
        StoreError(format!("archive illisible ({e})"))
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub struct LibraryContents {
    pub sets: Vec<SoundSet>,
    /// One line per folder that couldn't be read: its name and why.
    pub unreadable: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SetStore {
    directory: PathBuf,
}

impl SetStore {
    pub fn new(directory: PathBuf) -> Self {
        SetStore { directory }
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Every readable set; an unreadable folder (damaged, newer format…) is reported and
    /// skipped so it can't hide the others. Fails only if the library folder can't be listed.
    pub fn load_all(&self) -> Result<LibraryContents> {
        let mut contents = LibraryContents {
            sets: vec![],
            unreadable: vec![],
        };
        if !self.directory.exists() {
            return Ok(contents);
        }
        for entry in fs::read_dir(&self.directory)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some(PACKAGE_EXTENSION)
                || !path.is_dir()
            {
                continue;
            }
            match read_set(&fs::read(path.join(SET_FILE))?) {
                Ok(set) => contents.sets.push(set),
                Err(e) => contents
                    .unreadable
                    .push(format!("{} : {e}", file_name(&path))),
            }
        }
        Ok(contents)
    }

    /// Writes `Set.json` through a temporary file, so a crash never leaves half a set.
    pub fn save(&self, set: &SoundSet) -> Result<()> {
        let folder = self.package_path(set.id);
        fs::create_dir_all(&folder)?;
        let tmp = folder.join("Set.json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(set)?)?;
        fs::rename(tmp, folder.join(SET_FILE))?;
        Ok(())
    }

    /// Writes the set (its file, sounds and waveforms) as one `.soundset` zip at `destination`.
    pub fn export(&self, set: &SoundSet, destination: &Path) -> Result<()> {
        self.save(set)?;
        let folder = self.package_path(set.id);
        let tmp = destination.with_extension("soundset.tmp");
        let mut zip = zip::ZipWriter::new(fs::File::create(&tmp)?);
        // Sounds are already compressed: storing them is as small and much faster.
        let stored =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file(SET_FILE, SimpleFileOptions::default())?;
        zip.write_all(&fs::read(folder.join(SET_FILE))?)?;
        for name in FOLDERS {
            let Ok(entries) = fs::read_dir(folder.join(name)) else {
                continue;
            };
            for entry in entries {
                let path = entry?.path();
                if path.is_file() {
                    zip.start_file(format!("{name}/{}", file_name(&path)), stored)?;
                    io::copy(&mut fs::File::open(&path)?, &mut zip)?;
                }
            }
        }
        zip.finish()?;
        fs::rename(tmp, destination)?;
        Ok(())
    }

    /// The set a `.soundset` file holds, without adding it to the library.
    pub fn read_package(&self, source: &Path) -> Result<SoundSet> {
        let mut archive = zip::ZipArchive::new(fs::File::open(source)?)?;
        let mut data = Vec::new();
        archive
            .by_name(SET_FILE)
            .map_err(|_| StoreError(format!("{SET_FILE} manquant")))?
            .read_to_end(&mut data)?;
        read_set(&data)
    }

    /// Adds a copy of an exported `.soundset` file to the library, with fresh identifiers so
    /// importing the same file twice gives two independent sets.
    pub fn import_package(&self, source: &Path) -> Result<SoundSet> {
        let set = self.read_package(source)?.with_fresh_identifiers();
        let folder = self.package_path(set.id);
        let unpacked = unpack(source, &folder);
        if let Err(e) = unpacked.and_then(|()| self.save(&set)) {
            let _ = fs::remove_dir_all(&folder);
            return Err(e);
        }
        Ok(set)
    }

    /// Deletes the set's folder, including its copied sounds.
    pub fn delete(&self, set_id: Uuid) -> Result<()> {
        let folder = self.package_path(set_id);
        if folder.exists() {
            fs::remove_dir_all(folder)?;
        }
        Ok(())
    }

    /// Copies an audio file into the set's folder so the set no longer depends on the original.
    pub fn import_sound(&self, source: &Path, tile: &Tile, set_id: Uuid) -> Result<()> {
        let destination = self.sound_path(tile, set_id);
        fs::create_dir_all(destination.parent().expect("sounds live in a folder"))?;
        // Copy the real file behind a link, not the link itself.
        fs::copy(fs::canonicalize(source)?, destination)?;
        Ok(())
    }

    /// Deletes the copy of a tile's sound and its waveforms; a copy already gone is not an error.
    pub fn remove_sound(&self, tile: &Tile, set_id: Uuid) -> Result<()> {
        let sound = self.sound_path(tile, set_id);
        if sound.exists() {
            fs::remove_file(sound)?;
        }
        let waveforms = self.package_path(set_id).join("Waveforms");
        if let Ok(entries) = fs::read_dir(waveforms) {
            let prefix = tile.file_id.to_string();
            for entry in entries {
                let path = entry?.path();
                if file_name(&path).starts_with(&prefix) {
                    fs::remove_file(path)?;
                }
            }
        }
        Ok(())
    }

    /// Peaks saved for the tile's sound at this resolution: the sound file never changes, so
    /// its id is enough to find them.
    pub fn saved_waveform(&self, tile: &Tile, set_id: Uuid, bins: usize) -> Option<Vec<Peak>> {
        waveform::file::decode(&fs::read(self.waveform_path(tile, set_id, bins)).ok()?)
    }

    pub fn save_waveform(
        &self,
        peaks: &[Peak],
        tile: &Tile,
        set_id: Uuid,
        bins: usize,
    ) -> Result<()> {
        let path = self.waveform_path(tile, set_id, bins);
        fs::create_dir_all(path.parent().expect("waveforms live in a folder"))?;
        fs::write(path, waveform::file::encode(peaks))?;
        Ok(())
    }

    pub fn sound_path(&self, tile: &Tile, set_id: Uuid) -> PathBuf {
        let extension = Path::new(&tile.original_file_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        let name = if extension.is_empty() {
            tile.file_id.to_string()
        } else {
            format!("{}.{extension}", tile.file_id)
        };
        self.package_path(set_id).join("Sounds").join(name)
    }

    fn waveform_path(&self, tile: &Tile, set_id: Uuid, bins: usize) -> PathBuf {
        self.package_path(set_id)
            .join("Waveforms")
            .join(format!("{}-{bins}.peaks", tile.file_id))
    }

    fn package_path(&self, set_id: Uuid) -> PathBuf {
        self.directory.join(format!("{set_id}.{PACKAGE_EXTENSION}"))
    }
}

fn read_set(data: &[u8]) -> Result<SoundSet> {
    let set: SoundSet = serde_json::from_slice(data)?;
    if set.format_version > SoundSet::FORMAT_VERSION {
        return Err(StoreError(format!(
            "« {} » a été enregistré par une version plus récente de Soundboard (format {}).",
            set.title, set.format_version
        )));
    }
    Ok(set)
}

/// Extracts the sounds and waveforms of an archive. Only `Sounds/<file>` and
/// `Waveforms/<file>` entries are accepted: a path climbing out of the folder could otherwise
/// write anywhere on the disk.
fn unpack(source: &Path, folder: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(fs::File::open(source)?)?;
    let mut written: u64 = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_owned();
        if entry.is_dir() || name == SET_FILE {
            continue;
        }
        let parts: Vec<&str> = name.split('/').collect();
        let [dir, file] = parts[..] else {
            return Err(unexpected(&name));
        };
        let safe = FOLDERS.contains(&dir)
            && !file.is_empty()
            && !file.contains(['\\', ':'])
            && file != "."
            && file != "..";
        if !safe {
            return Err(unexpected(&name));
        }
        let target = folder.join(dir).join(file);
        fs::create_dir_all(target.parent().expect("inside a folder"))?;
        let mut out = fs::File::create(&target)?;
        let mut limited = (&mut entry).take(MAX_IMPORT_BYTES - written + 1);
        written += io::copy(&mut limited, &mut out)?;
        if written > MAX_IMPORT_BYTES {
            return Err(StoreError(
                "le set est trop volumineux une fois décompressé.".into(),
            ));
        }
    }
    Ok(())
}

fn unexpected(name: &str) -> StoreError {
    StoreError(format!(
        "le set contient un fichier inattendu (« {name} ») : il n’est pas importé par sécurité."
    ))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::tile;

    fn store() -> (tempfile::TempDir, SetStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SetStore::new(dir.path().join("Sets"));
        (dir, store)
    }

    fn set_with_sound(store: &SetStore, dir: &Path) -> SoundSet {
        let source = dir.join("bip.wav");
        fs::write(&source, b"RIFF fake").unwrap();
        let mut set = SoundSet::new("Effets");
        let t = tile("bip");
        store.import_sound(&source, &t, set.id).unwrap();
        set.tiles.push(t);
        store.save(&set).unwrap();
        set
    }

    #[test]
    fn saves_and_loads() {
        let (dir, store) = store();
        let set = set_with_sound(&store, dir.path());
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.sets, [set]);
        assert!(loaded.unreadable.is_empty());
    }

    #[test]
    fn unreadable_and_newer_sets_are_reported_not_fatal() {
        let (dir, store) = store();
        set_with_sound(&store, dir.path());
        let broken = store.directory().join("x.soundset");
        fs::create_dir_all(&broken).unwrap();
        fs::write(broken.join(SET_FILE), b"{").unwrap();
        let mut newer = SoundSet::new("Futur");
        newer.format_version = 99;
        store.save(&newer).unwrap();
        let loaded = store.load_all().unwrap();
        assert_eq!(loaded.sets.len(), 1);
        assert_eq!(loaded.unreadable.len(), 2);
    }

    #[test]
    fn export_then_import_gives_an_independent_copy() {
        let (dir, store) = store();
        let set = set_with_sound(&store, dir.path());
        store
            .save_waveform(
                &[Peak {
                    low: 0.0,
                    high: 1.0,
                }],
                &set.tiles[0],
                set.id,
                600,
            )
            .unwrap();
        let file = dir.path().join("Effets.soundset");
        store.export(&set, &file).unwrap();
        assert_eq!(store.read_package(&file).unwrap().id, set.id);

        let imported = store.import_package(&file).unwrap();
        assert_ne!(imported.id, set.id);
        assert_ne!(imported.tiles[0].id, set.tiles[0].id);
        let sound = store.sound_path(&imported.tiles[0], imported.id);
        assert_eq!(fs::read(sound).unwrap(), b"RIFF fake");
        assert!(store
            .saved_waveform(&imported.tiles[0], imported.id, 600)
            .is_some());
        assert_eq!(store.load_all().unwrap().sets.len(), 2);
    }

    #[test]
    fn archives_escaping_their_folder_are_refused() {
        let (dir, store) = store();
        let file = dir.path().join("evil.soundset");
        let mut zip = zip::ZipWriter::new(fs::File::create(&file).unwrap());
        zip.start_file(SET_FILE, SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&SoundSet::new("Evil")).unwrap())
            .unwrap();
        zip.start_file("Sounds/../../evil.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let error = store.import_package(&file).unwrap_err();
        assert!(error.0.contains("inattendu"), "{error}");
        assert!(
            store.load_all().unwrap().sets.is_empty(),
            "nothing is left behind"
        );
    }

    #[test]
    fn removing_a_sound_removes_its_waveforms() {
        let (dir, store) = store();
        let set = set_with_sound(&store, dir.path());
        let t = &set.tiles[0];
        store.save_waveform(&[], t, set.id, 600).unwrap();
        store.remove_sound(t, set.id).unwrap();
        assert!(!store.sound_path(t, set.id).exists());
        assert!(store.saved_waveform(t, set.id, 600).is_none());
        store.remove_sound(t, set.id).unwrap();
        store.delete(set.id).unwrap();
        assert!(store.load_all().unwrap().sets.is_empty());
    }
}
