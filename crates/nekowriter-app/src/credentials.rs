//! OS 凭据库边界。
//!
//! API key 永远不进入 SQLite、模型请求快照、SSE 事件或 tracing 字段。这里只以
//! 随机 `credential_ref` 作为 Windows Credential Manager 中同一条凭据的 username。

use keyring::v1::Entry;
use thiserror::Error;

const CREDENTIAL_SERVICE: &str = "NekoWriter";

#[derive(Debug, Error)]
pub enum CredentialStoreError {
    #[error("API key 不能为空")]
    EmptySecret,
    #[error("凭据引用无效")]
    InvalidReference,
    #[error("本机凭据库不可用或操作失败")]
    Unavailable,
    #[error("未找到对应的模型凭据")]
    Missing,
    #[error("凭据库任务未能完成")]
    TaskJoin,
}

/// 对 Windows Credential Manager（以及其他受支持平台的等价安全存储）的极小包装。
#[derive(Clone, Copy, Debug, Default)]
pub struct OsCredentialStore;

impl OsCredentialStore {
    pub fn write_api_key(
        self,
        credential_ref: &str,
        api_key: &str,
    ) -> Result<(), CredentialStoreError> {
        if api_key.trim().is_empty() {
            return Err(CredentialStoreError::EmptySecret);
        }
        self.entry(credential_ref)?
            .set_password(api_key)
            .map_err(|_| CredentialStoreError::Unavailable)
    }

    pub fn read_api_key(self, credential_ref: &str) -> Result<String, CredentialStoreError> {
        self.entry(credential_ref)?
            .get_password()
            .map_err(|_| CredentialStoreError::Missing)
    }

    pub fn delete_api_key(self, credential_ref: &str) -> Result<(), CredentialStoreError> {
        self.entry(credential_ref)?
            .delete_credential()
            .map_err(|_| CredentialStoreError::Unavailable)
    }

    fn entry(self, credential_ref: &str) -> Result<Entry, CredentialStoreError> {
        if credential_ref.is_empty()
            || credential_ref.chars().count() > 256
            || credential_ref.chars().any(char::is_control)
        {
            return Err(CredentialStoreError::InvalidReference);
        }
        Entry::new(CREDENTIAL_SERVICE, credential_ref)
            .map_err(|_| CredentialStoreError::Unavailable)
    }
}

pub async fn write_api_key(
    credential_ref: String,
    api_key: String,
) -> Result<(), CredentialStoreError> {
    tokio::task::spawn_blocking(move || OsCredentialStore.write_api_key(&credential_ref, &api_key))
        .await
        .map_err(|_| CredentialStoreError::TaskJoin)?
}

pub async fn read_api_key(credential_ref: String) -> Result<String, CredentialStoreError> {
    tokio::task::spawn_blocking(move || OsCredentialStore.read_api_key(&credential_ref))
        .await
        .map_err(|_| CredentialStoreError::TaskJoin)?
}

pub async fn delete_api_key(credential_ref: String) -> Result<(), CredentialStoreError> {
    tokio::task::spawn_blocking(move || OsCredentialStore.delete_api_key(&credential_ref))
        .await
        .map_err(|_| CredentialStoreError::TaskJoin)?
}
