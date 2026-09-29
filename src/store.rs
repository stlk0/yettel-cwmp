//! Saved router profiles and settings under one application-wide lock.
use crate::{
    domain::{
        export::Export,
        profile::{Profile, ProfileV1, StoredProfile},
        serial::Serial,
    },
    error::{Error, Result},
};
use serde::Serialize;
use std::{
    fs::{self, DirBuilder, File, OpenOptions, TryLockError},
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Resolve the platform's application data directory.
pub fn default_state_dir() -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    let base = dirs::state_dir();
    #[cfg(target_os = "macos")]
    let base = dirs::data_dir();
    #[cfg(target_os = "windows")]
    let base = dirs::data_local_dir();
    base.map(|path| path.join("yettel-cwmp"))
        .ok_or(Error::Storage)
}

/// Hold the application lock while accessing profiles and saved settings.
pub struct Store {
    root: PathBuf,
    _lock: File,
}

impl Store {
    /// Create the root if needed and acquire its persistent application lock.
    /// The root is made absolute once, so every returned path is absolute and consistent;
    /// symlinks are not resolved and Windows paths keep their usual form.
    pub fn open(root: &Path) -> Result<Self> {
        let root = std::path::absolute(root).map_err(|error| Error::storage_io(&error))?;
        let root = root.as_path();
        private_dir(root)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(root.join(".lock"))
            .map_err(|error| Error::storage_io(&error))?;
        lock.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => Error::AlreadyRunning,
            TryLockError::Error(error) => Error::storage_io(&error),
        })?;
        Ok(Self {
            root: root.to_path_buf(),
            _lock: lock,
        })
    }

    /// List saved routers in serial order, including profiles with damaged contents.
    pub fn list(&self) -> Result<Vec<Serial>> {
        let entries = match fs::read_dir(self.root.join("devices")) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::storage_io(&error)),
        };
        let mut serials = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| Error::storage_io(&error))?;
            if !entry
                .file_type()
                .map_err(|error| Error::storage_io(&error))?
                .is_dir()
            {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Ok(serial) = name.parse::<Serial>() else {
                continue;
            };
            if serial.as_ref() == name
                && entry
                    .path()
                    .join("profile.json")
                    .try_exists()
                    .map_err(|error| Error::storage_io(&error))?
            {
                serials.push(serial);
            }
        }
        serials.sort();
        Ok(serials)
    }

    /// Read and validate a profile without rewriting its stored version.
    pub fn load(&self, serial: &Serial) -> Result<Profile> {
        let bytes = fs::read(self.directory(serial).join("profile.json")).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Error::ProfileMissing
            } else {
                Error::storage_io(&error)
            }
        })?;
        let stored: StoredProfile<serde_json::Value> =
            serde_json::from_slice(&bytes).map_err(|_| Error::ProfileInvalid)?;
        let profile = match stored.version {
            1 => serde_json::from_value::<ProfileV1>(stored.profile).map(Profile::from_v1),
            2 => serde_json::from_value::<Profile>(stored.profile),
            _ => return Err(Error::ProfileInvalid),
        }
        .map_err(|_| Error::ProfileInvalid)?;
        profile.validate(serial)?;
        Ok(profile)
    }

    /// Create a profile without overwriting a saved router's identity or key.
    pub fn create(&self, profile: &Profile) -> Result<Profile> {
        if self
            .directory(&profile.serial)
            .join("profile.json")
            .try_exists()
            .map_err(|error| Error::storage_io(&error))?
        {
            let existing = self.load(&profile.serial)?;
            if existing.router_mac != profile.router_mac {
                return Err(Error::ProfileConflict);
            }
            if existing.password != profile.password {
                return Err(Error::ProfileExists);
            }
            return Ok(existing);
        }
        self.save(profile)?;
        Ok(profile.clone())
    }

    /// Atomically save a validated profile in version 2 format.
    pub fn save(&self, profile: &Profile) -> Result<()> {
        let directory = self.directory(&profile.serial);
        private_dir(&directory)?;
        write_json(
            &directory.join("profile.json"),
            &StoredProfile {
                version: 2,
                profile,
            },
        )
    }

    /// Remove a router's directory, even when its profile is damaged.
    pub fn delete(&self, serial: &Serial) -> Result<()> {
        match fs::remove_dir_all(self.directory(serial)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Error::storage_io(&error)),
        }
    }

    /// Read complete saved settings after validating the corresponding profile.
    /// Damaged settings count as missing: receiving them again repairs the file, while
    /// deleting the router would also discard its management credentials.
    pub fn load_export(&self, serial: &Serial) -> Result<(PathBuf, Export)> {
        self.load(serial)?;
        let path = self.directory(serial).join("extracted-credentials.json");
        let bytes = fs::read(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                Error::NoSavedSettings
            } else {
                Error::storage_io(&error)
            }
        })?;
        let export: Export = serde_json::from_slice(&bytes).map_err(|_| Error::NoSavedSettings)?;
        if export.internet.username.expose().is_empty()
            || export.internet.password.expose().is_empty()
        {
            return Err(Error::NoSavedSettings);
        }
        Ok((path, export))
    }

    /// Atomically save captured internet settings in the router's directory.
    pub fn save_export(&self, serial: &Serial, export: &Export) -> Result<PathBuf> {
        let path = self.directory(serial).join("extracted-credentials.json");
        write_json(&path, export)?;
        Ok(path)
    }

    /// Return the saved-settings modification time without reading its contents.
    pub fn export_modified(&self, serial: &Serial) -> Result<Option<SystemTime>> {
        match fs::metadata(self.directory(serial).join("extracted-credentials.json")) {
            Ok(metadata) => metadata
                .modified()
                .map(Some)
                .map_err(|error| Error::storage_io(&error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Error::storage_io(&error)),
        }
    }

    fn directory(&self, serial: &Serial) -> PathBuf {
        self.root.join("devices").join(serial.as_ref())
    }
}

fn private_dir(path: &Path) -> Result<()> {
    let mut builder = DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(path)
        .map_err(|error| Error::storage_io(&error))
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or(Error::Storage)?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|error| Error::storage_io(&error))?;
    serde_json::to_writer_pretty(&mut temp, value).map_err(|error| {
        error
            .io_error_kind()
            .map(|kind| Error::storage_io(&io::Error::from(kind)))
            .unwrap_or(Error::Storage)
    })?;
    temp.write_all(b"\n")
        .and_then(|()| temp.as_file().sync_all())
        .map_err(|error| Error::storage_io(&error))?;
    temp.persist(path)
        .map_err(|error| Error::storage_io(&error.error))?;
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| Error::storage_io(&error))?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde::ser::SerializeSeq;

    struct FailsDuringSerialization;

    impl Serialize for FailsDuringSerialization {
        fn serialize<S: serde::Serializer>(
            &self,
            serializer: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            let mut sequence = serializer.serialize_seq(Some(2))?;
            sequence.serialize_element("synthetic value")?;
            Err(serde::ser::Error::custom("synthetic serialization failure"))
        }
    }

    #[test]
    fn failed_serialization_preserves_previous_file_and_cleans_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("profile.json");
        let previous = b"synthetic previous contents\n";
        fs::write(&path, previous).unwrap();
        assert!(write_json(&path, &FailsDuringSerialization).is_err());
        assert_eq!(fs::read(&path).unwrap(), previous);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_replacement_preserves_destination_and_cleans_temporary_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("occupied");
        fs::create_dir(&path).unwrap();
        let previous = path.join("previous.json");
        fs::write(&previous, b"synthetic previous contents\n").unwrap();
        assert!(write_json(&path, &vec!["synthetic new contents"]).is_err());
        assert_eq!(
            fs::read(previous).unwrap(),
            b"synthetic previous contents\n"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
