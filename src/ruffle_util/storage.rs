use ruffle_core::backend::storage::StorageBackend;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub struct DiskStorageBackend {
    shared_objects_path: PathBuf,
}

impl DiskStorageBackend {
    pub fn new(shared_objects_path: PathBuf) -> Self {
DiskStorageBackend {
            shared_objects_path,
        }
    }

    /// Verifies that the path contains no `..` components to prevent accessing files outside of the Ruffle directory.
    fn is_path_allowed(path: &Path) -> bool {
        path.components().all(|c| c != Component::ParentDir)
    }

    fn get_shared_object_path(&self, name: &str) -> PathBuf {
        self.shared_objects_path.join(format!("{name}.sol"))
    }
}

impl StorageBackend for DiskStorageBackend {
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        let path = self.get_shared_object_path(name);
        if !Self::is_path_allowed(&path) {
            return None;
        }
        match std::fs::read(path) {
            Ok(data) => Some(data),
            Err(e) => {
                tracing::warn!("Unable to read file \"{}\": {:?}", name, e);
                None
            }
        }
    }

    fn get_size(&self, name: &str) -> Option<usize> {
        let path = self.get_shared_object_path(name);
        if !Self::is_path_allowed(&path) {
            return None;
        }
        match std::fs::metadata(path) {
            Ok(data) => data.len().try_into().ok(),
            Err(e) => {
                tracing::warn!("Unable to fetch metadata of file \"{}\": {:?}", name, e);
                None
            }
        }
    }

    fn put(&mut self, name: &str, value: &[u8]) -> bool {
        // Create a base dir if one doesn't exist yet
        // important: wait to do this until the flash content actually tries to save something, so
        // we don't clutter up the workspace with empty dirs
        if !self.shared_objects_path.exists() {
            tracing::info!("Creating storage dir");
            if let Err(r) = fs::create_dir_all(&self.shared_objects_path) {
                tracing::warn!("Unable to create storage dir {}", r);
            }
        }
        let path = self.get_shared_object_path(name);
        if !Self::is_path_allowed(&path) {
            return false;
        }
        if let Some(parent_dir) = path.parent() {
            if !parent_dir.exists() {
                if let Err(r) = fs::create_dir_all(parent_dir) {
                    tracing::warn!("Unable to create storage dir {}", r);
                    return false;
                }
            }
        }

        match File::create(path) {
            Ok(mut file) => {
                if let Err(r) = file.write_all(value) {
                    tracing::warn!("Unable to write file content {:?}", r);
                    false
                } else {
                    true
                }
            }
            Err(r) => {
                tracing::warn!("Unable to save file {:?}", r);
                false
            }
        }
    }

    fn remove_key(&mut self, name: &str) {
        let path = self.get_shared_object_path(name);
        if !Self::is_path_allowed(&path) {
            return;
        }
        let _ = fs::remove_file(path);
    }
}
