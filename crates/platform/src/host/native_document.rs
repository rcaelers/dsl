//! Native file access and file-dialog mechanisms.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::{DocumentError, FileOpenDialog, FileSaveDialog};

/// Native filesystem and file-dialog mechanisms.
#[derive(Clone, Copy, Default)]
pub struct NativeDocumentHost;

impl NativeDocumentHost {
    /// Creates a native document host.
    pub fn new() -> Self {
        Self
    }

    /// Reports whether a host path exists.
    pub fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    /// Reports whether a host path names a directory.
    pub fn is_directory(&self, path: &Path) -> bool {
        path.is_dir()
    }

    /// Opens a native file-selection dialog.
    pub fn choose_open_file(&self, request: FileOpenDialog) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(request.title);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        for filter in request.filters {
            dialog = dialog.add_filter(filter.name, &filter.extensions);
        }
        dialog.pick_file()
    }

    /// Opens a native file-destination dialog.
    pub fn choose_save_file(&self, request: FileSaveDialog) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new()
            .set_title(request.title)
            .set_file_name(request.default_file_name);
        if let Some(directory) = request.initial_directory {
            dialog = dialog.set_directory(directory);
        }
        for filter in request.filters {
            dialog = dialog.add_filter(filter.name, &filter.extensions);
        }
        dialog.save_file()
    }

    /// Opens a native directory-selection dialog.
    pub fn choose_directory(
        &self,
        title: &str,
        initial_directory: Option<&Path>,
    ) -> Option<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some(directory) = initial_directory {
            dialog = dialog.set_directory(directory);
        }
        dialog.pick_folder()
    }

    /// Reads all bytes from a host path.
    pub fn read(&self, path: &Path) -> Result<Vec<u8>, DocumentError> {
        std::fs::read(path).map_err(|error| DocumentError::Read {
            path: path.to_owned(),
            source: Box::new(error),
        })
    }

    /// Reads a host path when it exists.
    pub fn read_optional(&self, path: &Path) -> Result<Option<Vec<u8>>, DocumentError> {
        match std::fs::read(path) {
            Ok(contents) => Ok(Some(contents)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(DocumentError::Read {
                path: path.to_owned(),
                source: Box::new(error),
            }),
        }
    }

    /// Writes all bytes to a host path.
    pub fn write(&self, path: &Path, contents: &[u8]) -> Result<(), DocumentError> {
        std::fs::write(path, contents).map_err(|error| DocumentError::write(path, error))
    }

    /// Creates the parent directories required by a host path.
    pub fn create_parent_directories(&self, path: &Path) -> Result<(), DocumentError> {
        let Some(parent) = path.parent() else {
            return Err(DocumentError::MissingParent {
                path: path.to_owned(),
            });
        };
        std::fs::create_dir_all(parent).map_err(|error| DocumentError::CreateParent {
            path: parent.to_owned(),
            source: Box::new(error),
        })
    }

    /// Resolves a configuration-file path under an application namespace.
    pub fn configuration_file(&self, application_id: &str, name: &str) -> PathBuf {
        application_config_directory(application_id)
            .unwrap_or_else(|| std::env::temp_dir().join(application_id))
            .join(name)
    }
}

fn application_config_directory(application_id: &str) -> Option<PathBuf> {
    std::cfg_select! {
        target_os = "macos" => std::env::var_os("HOME").map(PathBuf::from).map(|home| {
            home.join("Library")
                .join("Application Support")
                .join(application_id)
        }),
        target_os = "windows" => std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|directory| directory.join(application_id)),
        _ => std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config"))
            })
            .map(|directory| directory.join(application_id)),
    }
}

#[cfg(test)]
mod native_document_tests {
    use super::NativeDocumentHost;

    #[test]
    fn configuration_path_uses_the_application_namespace() {
        let path =
            NativeDocumentHost::new().configuration_file("example-application", "settings.json");

        assert_eq!(path.file_name().unwrap(), "settings.json");
        assert_eq!(
            path.parent().unwrap().file_name().unwrap(),
            "example-application"
        );
    }
}
