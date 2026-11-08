use crate::{
    office_store::{Post, Provider},
    versioned_file::{StoredFile, VersionedDocument},
};
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, path::PathBuf};
use time::OffsetDateTime;

pub const ACCOUNTS_VERSION: u32 = 1;

pub type AccountsFile = StoredFile<AccountsDocument>;

/// Global catalog location: `%APPDATA%\UACode\office\accounts.json` on Windows.
pub fn accounts_path() -> PathBuf {
    paths::config_dir().join("office").join("accounts.json")
}

/// Parent of every credential folder (`CLAUDE_CONFIG_DIR` / `CODEX_HOME`).
pub fn credentials_root() -> PathBuf {
    paths::config_dir().join("accounts")
}

pub fn default_folder(provider: Provider) -> String {
    format!("{}-default", provider.slug())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: String,
    pub provider: Provider,
    pub name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Only set for imported accounts, which keep their pre-existing folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

impl Account {
    /// Derived from the id rather than the name so renaming never moves credentials.
    pub fn folder_name(&self) -> String {
        match &self.folder {
            Some(folder) => folder.clone(),
            None => {
                let short_id: String = self.id.chars().take(8).collect();
                format!("{}-{short_id}", self.provider.slug())
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountsDocument {
    pub version: u32,
    pub accounts: Vec<Account>,
}

impl Default for AccountsDocument {
    fn default() -> Self {
        Self {
            version: ACCOUNTS_VERSION,
            accounts: Vec::new(),
        }
    }
}

impl VersionedDocument for AccountsDocument {
    const VERSION: u32 = ACCOUNTS_VERSION;
    const FILE_NAME: &'static str = "accounts.json";

    fn version(&self) -> u32 {
        self.version
    }

    fn validate(&self) -> Result<()> {
        let mut identifiers = HashSet::new();
        let mut names = HashSet::new();
        let mut folders = HashSet::new();
        for account in &self.accounts {
            ensure!(!account.id.trim().is_empty(), "Cuenta sin identificador");
            ensure!(
                identifiers.insert(account.id.as_str()),
                "Identificador de cuenta duplicado: {}",
                account.id
            );
            validate_name(&account.name)?;
            ensure!(
                names.insert((account.provider, account.name.to_lowercase())),
                "Ya existe una cuenta de {} llamada «{}»",
                account.provider.label(),
                account.name
            );
            // Also covers folders derived from a hand-edited id.
            let folder = account.folder_name();
            validate_folder(&folder)?;
            ensure!(
                Provider::ALL
                    .iter()
                    .all(|provider| folder != default_folder(*provider)),
                "La carpeta {folder} está reservada para la cuenta por defecto"
            );
            ensure!(
                folders.insert(folder.to_lowercase()),
                "Dos cuentas usan la carpeta {folder}"
            );
        }
        Ok(())
    }
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.trim().is_empty(),
        "El nombre de la cuenta es obligatorio"
    );
    ensure!(
        name == name.trim(),
        "El nombre de la cuenta no puede empezar ni acabar en espacio"
    );
    Ok(())
}

fn validate_folder(folder: &str) -> Result<()> {
    let is_plain_name =
        !folder.is_empty() && folder != "." && folder != ".." && !folder.contains(['/', '\\', ':']);
    ensure!(is_plain_name, "Carpeta de cuenta no válida: {folder}");
    Ok(())
}

/// How a post's account reference resolves against the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountChoice<'a> {
    Default,
    Account(&'a Account),
    /// The referenced id is missing or belongs to another provider; the reference is kept.
    Missing,
}

impl AccountChoice<'_> {
    pub fn label(&self) -> String {
        match self {
            Self::Default => "Por defecto".into(),
            Self::Account(account) => account.name.clone(),
            Self::Missing => "Sin cuenta".into(),
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: Vec<String>,
    pub skipped: Vec<String>,
}

impl AccountsDocument {
    pub fn accounts_for(&self, provider: Provider) -> impl Iterator<Item = &Account> {
        self.accounts
            .iter()
            .filter(move |account| account.provider == provider)
    }

    pub fn resolve(&self, post: &Post) -> AccountChoice<'_> {
        let Some(id) = &post.account else {
            return AccountChoice::Default;
        };
        match self.accounts.iter().find(|account| &account.id == id) {
            Some(account) if account.provider == post.provider => AccountChoice::Account(account),
            _ => AccountChoice::Missing,
        }
    }

    pub fn add(&mut self, provider: Provider, name: &str, now: OffsetDateTime) -> Result<Account> {
        let name = name.trim();
        self.check_name_available(provider, name)?;
        let account = Account {
            id: uuid::Uuid::new_v4().to_string(),
            provider,
            name: name.to_owned(),
            created_at: now,
            folder: None,
        };
        self.accounts.push(account.clone());
        self.validate()?;
        Ok(account)
    }

    fn check_name_available(&self, provider: Provider, name: &str) -> Result<()> {
        validate_name(name)?;
        let lowercase = name.to_lowercase();
        if self
            .accounts_for(provider)
            .any(|account| account.name.to_lowercase() == lowercase)
        {
            bail!(
                "Ya existe una cuenta de {} llamada «{name}»",
                provider.label()
            );
        }
        Ok(())
    }

    /// Adds catalog entries for credential folders that exist on disk but are not referenced yet.
    /// Folders are kept where they are; nothing is moved or renamed.
    pub fn import_folders(
        &mut self,
        folder_names: impl IntoIterator<Item = String>,
        now: OffsetDateTime,
    ) -> ImportReport {
        let mut report = ImportReport::default();
        let mut folder_names: Vec<String> = folder_names.into_iter().collect();
        folder_names.sort();
        for folder in folder_names {
            let Some(provider) = Provider::ALL
                .into_iter()
                .find(|provider| folder.starts_with(&format!("{}-", provider.slug())))
            else {
                report
                    .skipped
                    .push(format!("{folder}: proveedor desconocido"));
                continue;
            };
            if Provider::ALL
                .iter()
                .any(|provider| folder == default_folder(*provider))
            {
                continue;
            }
            let lowercase = folder.to_lowercase();
            if self
                .accounts
                .iter()
                .any(|account| account.folder_name().to_lowercase() == lowercase)
            {
                continue;
            }
            if let Err(error) =
                validate_folder(&folder).and_then(|()| self.check_name_available(provider, &folder))
            {
                report.skipped.push(format!("{folder}: {error:#}"));
                continue;
            }
            self.accounts.push(Account {
                id: uuid::Uuid::new_v4().to_string(),
                provider,
                name: folder.clone(),
                created_at: now,
                folder: Some(folder.clone()),
            });
            report.imported.push(folder);
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::versioned_file::{load, save};
    use fs::Fs;
    use std::path::Path;

    fn now() -> OffsetDateTime {
        OffsetDateTime::UNIX_EPOCH
    }

    fn post(provider: Provider, account: Option<String>) -> Post {
        Post {
            id: "post".into(),
            name: "Backend".into(),
            role: "backend".into(),
            provider,
            account,
            model: String::new(),
            instructions: String::new(),
            archived: false,
        }
    }

    fn sample() -> Result<AccountsDocument> {
        let mut document = AccountsDocument::default();
        document.add(Provider::Claude, "Empresa", now())?;
        document.import_folders(["claude-b".to_string()], now());
        Ok(document)
    }

    #[test]
    fn serialization_round_trip() -> Result<()> {
        let expected = sample()?;
        let encoded = serde_json::to_string(&expected)?;
        let actual: AccountsDocument = serde_json::from_str(&encoded)?;
        assert_eq!(actual, expected);
        assert!(!encoded.contains("\"folder\":null"));
        Ok(())
    }

    #[test]
    fn invalid_json_is_read_only_with_line() {
        let file = AccountsFile::decode(Some("{\n  \"version\": 1,\n  \"accounts\": [}\n".into()));
        assert!(file.is_read_only());
        assert!(
            file.warning
                .as_deref()
                .is_some_and(|warning| warning.contains("line 3"))
        );
    }

    #[test]
    fn future_version_is_read_only() {
        let file = AccountsFile::decode(Some(r#"{"version": 7, "accounts": {}}"#.into()));
        assert!(file.is_read_only());
        assert!(
            file.warning
                .as_deref()
                .is_some_and(|warning| warning.contains('7'))
        );
    }

    #[test]
    fn unknown_fields_such_as_tokens_are_rejected() {
        let text = r#"{"version":1,"accounts":[{"id":"a","provider":"claude","name":"A","created_at":"2026-11-08T10:00:00Z","token":"x"}]}"#;
        assert!(AccountsFile::decode(Some(text.into())).is_read_only());
    }

    #[test]
    fn duplicate_names_are_rejected_per_provider_ignoring_case() -> Result<()> {
        let mut document = AccountsDocument::default();
        document.add(Provider::Claude, "Empresa", now())?;
        let error = document
            .add(Provider::Claude, "  EMPRESA ", now())
            .err()
            .map(|error| error.to_string());
        assert!(error.is_some_and(|error| error.contains("Ya existe")));
        document.add(Provider::Codex, "Empresa", now())?;
        assert_eq!(document.accounts.len(), 2);
        Ok(())
    }

    #[test]
    fn folder_derives_from_id_not_name() -> Result<()> {
        let mut document = AccountsDocument::default();
        let account = document.add(Provider::Codex, "Personal", now())?;
        let short_id: String = account.id.chars().take(8).collect();
        assert_eq!(account.folder_name(), format!("codex-{short_id}"));
        document.accounts[0].name = "Renombrada".into();
        assert_eq!(document.accounts[0].folder_name(), account.folder_name());
        Ok(())
    }

    #[test]
    fn missing_or_foreign_account_resolves_to_missing_without_clearing() -> Result<()> {
        let document = sample()?;
        let claude_id = document.accounts[0].id.clone();
        assert_eq!(
            document.resolve(&post(Provider::Claude, None)),
            AccountChoice::Default
        );
        assert_eq!(
            document.resolve(&post(Provider::Claude, Some(claude_id.clone()))),
            AccountChoice::Account(&document.accounts[0])
        );
        let missing = post(Provider::Claude, Some("borrada".into()));
        assert_eq!(document.resolve(&missing), AccountChoice::Missing);
        assert_eq!(missing.account.as_deref(), Some("borrada"));
        assert_eq!(
            document.resolve(&post(Provider::Codex, Some(claude_id))),
            AccountChoice::Missing
        );
        Ok(())
    }

    #[test]
    fn changing_provider_resets_account_to_default() {
        let mut post = post(Provider::Claude, Some("id".into()));
        post.set_provider(Provider::Claude);
        assert_eq!(post.account.as_deref(), Some("id"));
        post.set_provider(Provider::Codex);
        assert_eq!(post.account, None);
    }

    #[test]
    fn import_keeps_existing_folders_and_skips_defaults() -> Result<()> {
        let mut document = AccountsDocument::default();
        document.add(Provider::Claude, "claude-c", now())?;
        let folders = [
            "claude-default",
            "codex-default",
            "claude-b",
            "claude-c",
            "gemini-x",
        ]
        .map(String::from);
        let report = document.import_folders(folders.clone(), now());
        assert_eq!(report.imported, vec!["claude-b".to_string()]);
        assert_eq!(report.skipped.len(), 2);
        let imported = document
            .accounts
            .iter()
            .find(|account| account.folder.as_deref() == Some("claude-b"));
        assert!(imported.is_some_and(|account| account.folder_name() == "claude-b"));
        document.validate()?;

        let again = document.import_folders(folders, now());
        assert!(again.imported.is_empty());
        Ok(())
    }

    #[test]
    fn reserved_and_unsafe_folders_are_rejected() {
        for folder in ["claude-default", "..", "a/b", "c:\\x"] {
            let document = AccountsDocument {
                accounts: vec![Account {
                    id: "a".into(),
                    provider: Provider::Claude,
                    name: "A".into(),
                    created_at: now(),
                    folder: Some(folder.into()),
                }],
                ..AccountsDocument::default()
            };
            assert!(document.validate().is_err(), "{folder}");
        }
        let document = AccountsDocument {
            accounts: vec![Account {
                id: "../x".into(),
                provider: Provider::Claude,
                name: "A".into(),
                created_at: now(),
                folder: None,
            }],
            ..AccountsDocument::default()
        };
        assert!(
            document.validate().is_err(),
            "a derived folder must be validated too"
        );
    }

    // `gpui::test` discards the returned value, so failures must surface through `expect`.
    #[gpui::test]
    async fn missing_catalog_loads_empty_without_writing(cx: &mut gpui::TestAppContext) {
        let fs = fs::FakeFs::new(cx.background_executor.clone());
        check_missing_catalog(fs.as_ref())
            .await
            .expect("missing accounts.json should load without side effects");
    }

    async fn check_missing_catalog(fs: &dyn Fs) -> Result<()> {
        let path = Path::new("/config/office/accounts.json");
        let file: AccountsFile = load(fs, path).await?;
        assert_eq!(file.document, AccountsDocument::default());
        assert!(fs.metadata(Path::new("/config/office")).await?.is_none());
        Ok(())
    }

    #[gpui::test]
    async fn save_rejects_external_edits(cx: &mut gpui::TestAppContext) {
        let fs = fs::FakeFs::new(cx.background_executor.clone());
        check_external_edit(fs.as_ref())
            .await
            .expect("save and external edit checks should succeed");
    }

    async fn check_external_edit(fs: &dyn Fs) -> Result<()> {
        let path = Path::new("/config/office/accounts.json");
        let initial: AccountsFile = load(fs, path).await?;
        let saved = save(fs, path, &initial, sample()?).await?;
        assert_eq!(
            load::<AccountsDocument>(fs, path).await?.document,
            saved.document
        );
        fs.atomic_write(path.to_path_buf(), "{\"version\":1,\"accounts\":[]}".into())
            .await?;
        assert!(
            save(fs, path, &saved, saved.document.clone())
                .await
                .is_err()
        );
        Ok(())
    }
}
