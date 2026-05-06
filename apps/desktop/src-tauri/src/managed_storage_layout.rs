use std::path::PathBuf;
const RECORDINGS_DIR_NAME: &str = "recordings";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedStorageLayout {
    base_dir: PathBuf,
}

impl ManagedStorageLayout {
    pub(crate) fn from_save_directory(save_directory: &str) -> Self {
        Self {
            base_dir: PathBuf::from(save_directory),
        }
    }

    pub(crate) fn from_base_dir(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub(crate) fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    pub(crate) fn recordings_root(&self) -> PathBuf {
        self.base_dir.join(RECORDINGS_DIR_NAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn managed_storage_layout_uses_save_directory_as_base_dir() {
        let layout = ManagedStorageLayout::from_save_directory("/tmp/mnema-recordings");

        assert_eq!(layout.base_dir(), &PathBuf::from("/tmp/mnema-recordings"));
    }

    #[test]
    fn managed_storage_layout_keeps_database_out_of_segment_root() {
        let layout =
            ManagedStorageLayout::from_save_directory("/tmp/mnema-recordings/session-output");

        assert_eq!(
            layout.base_dir().parent(),
            Some(Path::new("/tmp/mnema-recordings/session-output"))
        );
        assert_eq!(
            layout
                .base_dir()
                .file_name()
                .and_then(|value| value.to_str()),
            Some("session-output")
        );
    }

    #[test]
    fn recordings_root_nests_under_save_directory() {
        let layout = ManagedStorageLayout::from_save_directory("/tmp/mnema-recordings");

        assert_eq!(
            layout.recordings_root(),
            PathBuf::from("/tmp/mnema-recordings").join("recordings")
        );
    }

    #[test]
    fn recordings_root_is_child_of_base_dir() {
        let layout = ManagedStorageLayout::from_save_directory("/tmp/mnema-recordings");
        let base_dir = layout.base_dir().clone();
        let recordings_root = layout.recordings_root();

        assert_eq!(recordings_root.parent(), Some(base_dir.as_path()));
    }
}
