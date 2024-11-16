
use ahash::{HashMap, HashMapExt};
use compact_str::{format_compact, CompactString, ToCompactString};

use super::FilePath;

#[derive(Debug)]
pub enum MetadataFile {
    Packages(FilePath),
    Sources(FilePath),
    DiffIndex(FilePath),
    DebianInstallerSumFile(FilePath),
    Other(FilePath)
}

impl MetadataFile {
    pub fn path(&self) -> &FilePath {
        match self {
            MetadataFile::Packages(file_path) |
            MetadataFile::Sources(file_path) |
            MetadataFile::DiffIndex(file_path) |
            MetadataFile::DebianInstallerSumFile(file_path) |
            MetadataFile::Other(file_path) => file_path
        }
    }

    pub fn path_mut(&mut self) -> &mut FilePath {
        match self {
            MetadataFile::Packages(file_path) |
            MetadataFile::Sources(file_path) |
            MetadataFile::DiffIndex(file_path) |
            MetadataFile::DebianInstallerSumFile(file_path) |
            MetadataFile::Other(file_path) => file_path
        }
    }

    pub fn prefix_with(&mut self, prefix: &str) {
        let prefix = FilePath(prefix.to_compact_string());

        let s = self.path_mut();

        *s = prefix.join(s.as_str());
    }

    pub fn is_index(&self) -> bool {
        !matches!(self, Self::Other(_))
    }

    pub fn extension(&self) -> Option<&str> {
        self.path().extension()
    }

    pub fn canonical_path(&self) -> FilePath {
        match self {
            MetadataFile::Sources(file_path) |
            MetadataFile::Packages(file_path) => {
                let stem = file_path.file_stem();
                let parent = file_path.parent().unwrap_or("");
                FilePath(format_compact!("{parent}/{stem}"))
            },
            MetadataFile::DebianInstallerSumFile(file_path) => {
                let path = file_path.parent().unwrap();
                FilePath(path.to_compact_string())
            },
            MetadataFile::DiffIndex(file_path) |
            MetadataFile::Other(file_path) => file_path.clone(),
        }
    }

    pub fn exists(&self) -> bool {
        self.path().exists()
    }
}

impl AsRef<str> for MetadataFile {
    fn as_ref(&self) -> &str {
        self.path().as_str()
    }
}

impl AsRef<FilePath> for MetadataFile {
    fn as_ref(&self) -> &FilePath {
        self.path()
    }
}

impl From<CompactString> for MetadataFile {
    fn from(value: CompactString) -> Self {
        let value = FilePath(value);

        if is_packages_file(&value) {
            return MetadataFile::Packages(value)
        }
        
        if is_sources_file(&value) {
            return MetadataFile::Sources(value)
        }

        if is_diff_index_file(&value) {
            return MetadataFile::DiffIndex(value)
        }
        
        if is_debian_installer_sumfile(&value) {
            return MetadataFile::DebianInstallerSumFile(value)
        }

        MetadataFile::Other(value)
    }
}

pub fn is_packages_file(path: &FilePath) -> bool {
    path.file_stem() == "Packages"
}

pub fn is_diff_index_file(path: &FilePath) -> bool {
    path.file_stem() == "Index"
}

pub fn is_debian_installer_sumfile(path: &FilePath) -> bool {
    path.file_stem().ends_with("SUMS") && path.parent().unwrap_or("").contains("installer-")
}

pub fn is_sources_file(path: &FilePath) -> bool {
    path.file_stem() == "Sources"
}

pub fn deduplicate_metadata(files: Vec<MetadataFile>) -> Vec<MetadataFile> {
    let mut map: HashMap<FilePath, MetadataFile> = HashMap::with_capacity(files.capacity() * 2);

    for file in files {
        let canonical = file.canonical_path();

        match &file {
            MetadataFile::Packages(..) |
            MetadataFile::Sources(..) => {
                if let Some(old) = map.get_mut(&canonical) {
                    if is_extension_preferred(old.extension(), file.extension()) {
                        *old = file;
                    }

                    continue
                }
            },
            MetadataFile::DebianInstallerSumFile(sum_file) => {
                if let Some(old_file) = map.get_mut(&canonical) {
                    let MetadataFile::DebianInstallerSumFile(old) = old_file else {
                        panic!("implementation error; non-sumfile being compared to sumfile")
                    };

                    if is_sumfile_preferred(old.file_name(), sum_file.file_name()) {
                        *old_file = file;
                    }

                    continue
                }
            },
            MetadataFile::DiffIndex(..) |
            MetadataFile::Other(..) => (),
        }

        map.insert(canonical, file);
    }

    map.into_values().collect()
}

fn is_extension_preferred(old: Option<&str>, new: Option<&str>) -> bool {
    matches!((old, new),
        (_, Some("gz")) |
        (_, Some("xz")) |
        (_, Some("bz2")) 
    )
}

fn is_sumfile_preferred(old: &str, new: &str) -> bool {
    matches!((old, new), 
        (_, "SHA512SUMS") |
        (_, "SHA256SUMS")
    )
}