use anyhow::{Context as _, Result, ensure};
use fs::Fs;
use serde::{Serialize, de::DeserializeOwned};
use std::path::Path;

pub trait VersionedDocument: Serialize + DeserializeOwned + Default + Clone {
    const VERSION: u32;
    const FILE_NAME: &'static str;

    fn version(&self) -> u32;
    fn validate(&self) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct StoredFile<T> {
    pub document: T,
    pub warning: Option<String>,
    pub(crate) original: Option<String>,
}

impl<T: VersionedDocument> StoredFile<T> {
    pub fn is_read_only(&self) -> bool {
        self.warning.is_some()
    }

    pub fn exists_on_disk(&self) -> bool {
        self.original.is_some()
    }

    pub(crate) fn decode(original: Option<String>) -> Self {
        let mut file = Self {
            document: T::default(),
            warning: None,
            original,
        };
        if let Some(text) = &file.original {
            match parse::<T>(text) {
                Ok(document) => file.document = document,
                Err(error) => {
                    file.warning = Some(format!("{} en solo lectura: {error:#}", T::FILE_NAME))
                }
            }
        }
        file
    }
}

fn parse<T: VersionedDocument>(text: &str) -> Result<T> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(version) = value.get("version").and_then(|version| version.as_u64()) {
        ensure!(
            version <= u64::from(T::VERSION),
            "versión {version} creada por una app más nueva; esta admite hasta {}",
            T::VERSION
        );
    }
    let document: T = serde_json::from_str(text)?;
    validate(&document)?;
    Ok(document)
}

fn validate<T: VersionedDocument>(document: &T) -> Result<()> {
    ensure!(
        document.version() == T::VERSION,
        "versión {} no compatible",
        document.version()
    );
    document.validate()
}

pub async fn load<T: VersionedDocument>(fs: &dyn Fs, path: &Path) -> Result<StoredFile<T>> {
    let original = if fs.metadata(path).await?.is_some() {
        Some(fs.load(path).await?)
    } else {
        None
    };
    Ok(StoredFile::decode(original))
}

/// Writes atomically, refusing when the previous read was read-only or the file changed on disk since.
pub async fn save<T: VersionedDocument>(
    fs: &dyn Fs,
    path: &Path,
    previous: &StoredFile<T>,
    document: T,
) -> Result<StoredFile<T>> {
    ensure!(
        !previous.is_read_only(),
        "{} está en solo lectura; corrígelo y pulsa Recargar",
        T::FILE_NAME
    );
    validate(&document)?;
    let current = load::<T>(fs, path).await?;
    ensure!(
        current.original == previous.original,
        "{} cambió en disco; pulsa Recargar antes de guardar",
        T::FILE_NAME
    );
    let text = serde_json::to_string_pretty(&document)? + "\n";
    let parent = path
        .parent()
        .with_context(|| format!("{} no tiene carpeta padre", T::FILE_NAME))?;
    fs.create_dir(parent).await?;
    fs.atomic_write(path.to_path_buf(), text.clone()).await?;
    Ok(StoredFile {
        document,
        warning: None,
        original: Some(text),
    })
}
